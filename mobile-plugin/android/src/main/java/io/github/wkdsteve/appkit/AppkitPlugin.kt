// The Android side of appkit's mobile basis (docs/MOBILE.md). The shell of an app (Rust) calls it; the page never does
// (the plugin names no command of the page). It holds what only the platform can hold or do:
//
// - the device keys in the Android Keystore (AND-05, MSEC-01): a login key, and a trigger key that signs only after
//   the biometrics or the device lock, bound by a CryptoObject; neither can be exported, the hub knows the public halves;
// - the QR code of a pairing, read with ZXing (no network, no analytics);
// - the folder of a vault cloned on the phone (the Storage Access Framework), read only and bounded;
// - UnifiedPush: the endpoint a distributor gives, and the notice for a message (the kind only, never content).
//
// Nothing is logged with content (AND-11). Every refusal is a short word the shell turns into a state of the page.

package io.github.wkdsteve.appkit

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.DocumentsContract
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.util.Base64
import androidx.activity.result.ActivityResult
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.zxing.client.android.Intents
import com.journeyapps.barcodescanner.ScanIntentResult
import com.journeyapps.barcodescanner.ScanOptions
import org.unifiedpush.android.connector.UnifiedPush
import java.math.BigInteger
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec

@InvokeArg
class KeyArgs {
    lateinit var alias: String
    var userAuth: Boolean = false
}

@InvokeArg
class SignArgs {
    lateinit var alias: String
    /** The message, base64 (standard). */
    lateinit var message: String
    var userAuth: Boolean = false
    var title: String = ""
    var subtitle: String = ""
}

@InvokeArg
class PathArgs {
    lateinit var path: String
    var max: Int = 0
}

@InvokeArg
class PushArgs {
    /** The VAPID public key of the hub, base64url of the uncompressed point. */
    lateinit var vapid: String
    lateinit var title: String
    lateinit var text: String
}

/** Rejects with a word, never with content. */
private fun Invoke.refuse(word: String) = reject(word, word)

private const val KEYSTORE = "AndroidKeyStore"
private const val PREFS = "appkit-mobile"
private const val TREE = "tree"
private const val MAX_ENTRIES = 2000
private const val MAX_DEPTH = 8

@TauriPlugin
class AppkitPlugin(private val activity: Activity) : Plugin(activity) {

    // ── device keys ────────────────────────────────────────────────────────

    private fun keyStore(): KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    /** A P-256 signing key under `alias`; with `userAuth` it signs only after the person proved themself. */
    private fun makeKey(alias: String, userAuth: Boolean, strongBox: Boolean) {
        val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
        if (userAuth) {
            spec.setUserAuthenticationRequired(true)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                // Each use, with a strong biometric or the device lock.
                spec.setUserAuthenticationParameters(
                    0,
                    KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL,
                )
            } else {
                @Suppress("DEPRECATION")
                spec.setUserAuthenticationValidityDurationSeconds(-1)
            }
        }
        if (strongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            spec.setIsStrongBoxBacked(true)
        }
        KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, KEYSTORE).apply {
            initialize(spec.build())
            generateKeyPair()
        }
    }

    /** SEC1 uncompressed (65 bytes) of the public key, lower-case hex. */
    private fun publicHex(alias: String): String? {
        val cert = keyStore().getCertificate(alias) ?: return null
        val key = cert.publicKey as ECPublicKey
        val out = ByteArray(65)
        out[0] = 4
        fixed32(key.w.affineX).copyInto(out, 1)
        fixed32(key.w.affineY).copyInto(out, 33)
        return out.joinToString("") { "%02x".format(it) }
    }

    @Command
    fun publicKey(invoke: Invoke) {
        val args = invoke.parseArgs(KeyArgs::class.java)
        try {
            publicHex(args.alias)?.let {
                invoke.resolve(JSObject().put("key", it))
                return
            }
            try {
                makeKey(args.alias, args.userAuth, true)
            } catch (e: StrongBoxUnavailableException) {
                makeKey(args.alias, args.userAuth, false)
            }
            val key = publicHex(args.alias) ?: return invoke.refuse("failed")
            invoke.resolve(JSObject().put("key", key))
        } catch (e: IllegalStateException) {
            // A key bound to the person needs a lock screen.
            invoke.refuse("no_lock")
        } catch (e: Exception) {
            invoke.refuse("failed")
        }
    }

    @Command
    fun deleteKey(invoke: Invoke) {
        val args = invoke.parseArgs(KeyArgs::class.java)
        try {
            val store = keyStore()
            if (store.containsAlias(args.alias)) store.deleteEntry(args.alias)
            if (store.containsAlias(args.alias)) invoke.refuse("failed") else invoke.resolve()
        } catch (e: Exception) {
            invoke.refuse("failed")
        }
    }

    @Command
    fun sign(invoke: Invoke) {
        val args = invoke.parseArgs(SignArgs::class.java)
        val message = try {
            Base64.decode(args.message, Base64.NO_WRAP)
        } catch (e: IllegalArgumentException) {
            return invoke.refuse("failed")
        }
        val key = try {
            keyStore().getKey(args.alias, null) as? PrivateKey
        } catch (e: Exception) {
            null
        } ?: return invoke.refuse("gone")
        val signature = Signature.getInstance("SHA256withECDSA")
        try {
            signature.initSign(key)
        } catch (e: android.security.keystore.KeyPermanentlyInvalidatedException) {
            return invoke.refuse("gone")
        } catch (e: Exception) {
            return invoke.refuse("failed")
        }
        if (!args.userAuth) {
            return try {
                signature.update(message)
                invoke.resolve(JSObject().put("signature", raw(signature.sign())))
            } catch (e: Exception) {
                invoke.refuse("failed")
            }
        }
        val host = activity as? FragmentActivity ?: return invoke.refuse("failed")
        host.runOnUiThread {
            val callback = object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    val bound = result.cryptoObject?.signature ?: return invoke.refuse("failed")
                    try {
                        bound.update(message)
                        invoke.resolve(JSObject().put("signature", raw(bound.sign())))
                    } catch (e: Exception) {
                        invoke.refuse("failed")
                    }
                }

                override fun onAuthenticationError(code: Int, text: CharSequence) {
                    invoke.refuse(
                        when (code) {
                            BiometricPrompt.ERROR_USER_CANCELED,
                            BiometricPrompt.ERROR_NEGATIVE_BUTTON,
                            BiometricPrompt.ERROR_CANCELED -> "cancelled"
                            BiometricPrompt.ERROR_NO_BIOMETRICS,
                            BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL -> "no_lock"
                            else -> "failed"
                        }
                    )
                }
            }
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle(args.title.ifEmpty { "Confirm" })
                .setSubtitle(args.subtitle)
                .setConfirmationRequired(true)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                info.setAllowedAuthenticators(
                    BiometricManager.Authenticators.BIOMETRIC_STRONG or
                        BiometricManager.Authenticators.DEVICE_CREDENTIAL
                )
            } else {
                info.setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                info.setNegativeButtonText("Cancel")
            }
            try {
                BiometricPrompt(host, ContextCompat.getMainExecutor(host), callback)
                    .authenticate(info.build(), BiometricPrompt.CryptoObject(signature))
            } catch (e: Exception) {
                invoke.refuse("failed")
            }
        }
    }

    // ── the QR code of a pairing ───────────────────────────────────────────

    @Command
    fun scanQr(invoke: Invoke) {
        // The hub draws the code with block characters in the terminal's text colour: on a dark terminal it is light
        // on dark (inverted). MIXED_SCAN reads both polarities, alternating frame by frame.
        val intent = ScanOptions()
            .addExtra(Intents.Scan.SCAN_TYPE, Intents.Scan.MIXED_SCAN)
            .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
            .setBeepEnabled(false)
            .setOrientationLocked(false)
            .setPrompt("")
            .createScanIntent(activity)
        startActivityForResult(invoke, intent, "scanned")
    }

    @ActivityCallback
    private fun scanned(invoke: Invoke, result: ActivityResult) {
        val read = ScanIntentResult.parseActivityResult(result.resultCode, result.data)
        val text = read.contents
        // Only the text of the code; the shell checks it (a pairing link of exactly one form).
        if (text == null || text.length > 512) invoke.resolve(JSObject().put("text", null as String?))
        else invoke.resolve(JSObject().put("text", text))
    }

    // ── a vault cloned on the phone ────────────────────────────────────────

    @Command
    fun pickFolder(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
        startActivityForResult(invoke, intent, "folderPicked")
    }

    @ActivityCallback
    private fun folderPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            return invoke.refuse("cancelled")
        }
        try {
            val resolver = activity.contentResolver
            // Drop the folder chosen before: the app holds at most one.
            prefs().getString(TREE, null)?.let { old ->
                try {
                    resolver.releasePersistableUriPermission(
                        Uri.parse(old),
                        Intent.FLAG_GRANT_READ_URI_PERMISSION,
                    )
                } catch (e: SecurityException) {
                }
            }
            resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            prefs().edit().putString(TREE, uri.toString()).apply()
            invoke.resolve(JSObject().put("picked", true))
        } catch (e: Exception) {
            invoke.refuse("failed")
        }
    }

    @Command
    fun forgetFolder(invoke: Invoke) {
        prefs().getString(TREE, null)?.let { old ->
            try {
                activity.contentResolver.releasePersistableUriPermission(
                    Uri.parse(old),
                    Intent.FLAG_GRANT_READ_URI_PERMISSION,
                )
            } catch (e: SecurityException) {
            }
        }
        prefs().edit().remove(TREE).apply()
        invoke.resolve()
    }

    private fun prefs() = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private class Entry(val id: String, val name: String, val dir: Boolean, val size: Long)

    private fun children(tree: Uri, parent: String): List<Entry> {
        val uri = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parent)
        val out = ArrayList<Entry>()
        activity.contentResolver.query(
            uri,
            arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_MIME_TYPE,
                DocumentsContract.Document.COLUMN_SIZE,
            ),
            null,
            null,
            null,
        )?.use { c ->
            while (c.moveToNext() && out.size < MAX_ENTRIES) {
                out.add(
                    Entry(
                        c.getString(0),
                        c.getString(1) ?: "",
                        c.getString(2) == DocumentsContract.Document.MIME_TYPE_DIR,
                        if (c.isNull(3)) 0 else c.getLong(3),
                    )
                )
            }
        }
        return out
    }

    /** The document id of `path` below the folder (names joined by `/`), walked name by name. */
    private fun find(tree: Uri, path: String): Entry? {
        val parts = if (path.isEmpty()) emptyList() else path.split('/')
        if (parts.size > MAX_DEPTH || parts.any { it.isEmpty() || it == "." || it == ".." }) return null
        var here = Entry(DocumentsContract.getTreeDocumentId(tree), "", true, 0)
        for (part in parts) {
            if (!here.dir) return null
            here = children(tree, here.id).firstOrNull { it.name == part } ?: return null
        }
        return here
    }

    private fun tree(): Uri? = prefs().getString(TREE, null)?.let { Uri.parse(it) }

    @Command
    fun listFolder(invoke: Invoke) {
        val args = invoke.parseArgs(PathArgs::class.java)
        val tree = tree() ?: return invoke.refuse("no_folder")
        try {
            val dir = find(tree, args.path)
            if (dir == null || !dir.dir) return invoke.refuse("missing")
            val list = JSArray()
            for (e in children(tree, dir.id)) {
                list.put(JSObject().put("name", e.name).put("dir", e.dir).put("size", e.size))
            }
            invoke.resolve(JSObject().put("entries", list))
        } catch (e: SecurityException) {
            invoke.refuse("no_folder")
        } catch (e: Exception) {
            invoke.refuse("failed")
        }
    }

    @Command
    fun readFile(invoke: Invoke) {
        val args = invoke.parseArgs(PathArgs::class.java)
        val tree = tree() ?: return invoke.refuse("no_folder")
        if (args.max <= 0) return invoke.refuse("failed")
        try {
            val file = find(tree, args.path)
            if (file == null || file.dir) return invoke.refuse("missing")
            if (file.size > args.max) return invoke.refuse("too_large")
            val uri = DocumentsContract.buildDocumentUriUsingTree(tree, file.id)
            val bytes = activity.contentResolver.openInputStream(uri)?.use { input ->
                val buf = ByteArray(args.max + 1)
                var n = 0
                while (n < buf.size) {
                    val r = input.read(buf, n, buf.size - n)
                    if (r < 0) break
                    n += r
                }
                if (n > args.max) null else buf.copyOf(n)
            } ?: return invoke.refuse("too_large")
            invoke.resolve(JSObject().put("text", String(bytes, Charsets.UTF_8)))
        } catch (e: SecurityException) {
            invoke.refuse("no_folder")
        } catch (e: Exception) {
            invoke.refuse("failed")
        }
    }

    // ── push ───────────────────────────────────────────────────────────────

    @Command
    fun pushRegister(invoke: Invoke) {
        val args = invoke.parseArgs(PushArgs::class.java)
        if (UnifiedPush.getDistributors(activity).isEmpty()) return invoke.refuse("no_distributor")
        Push.saveNotice(activity, args.title.take(80), args.text.take(200))
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(activity, android.Manifest.permission.POST_NOTIFICATIONS) !=
            android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            ActivityCompat.requestPermissions(
                activity,
                arrayOf(android.Manifest.permission.POST_NOTIFICATIONS),
                41,
            )
        }
        UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { ok ->
            if (!ok) {
                invoke.refuse("no_distributor")
            } else {
                try {
                    UnifiedPush.register(activity, vapid = args.vapid)
                    invoke.resolve()
                } catch (e: Exception) {
                    invoke.refuse("failed")
                }
            }
        }
    }

    @Command
    fun pushUnregister(invoke: Invoke) {
        UnifiedPush.unregister(activity)
        Push.forget(activity)
        invoke.resolve()
    }

    @Command
    fun pushState(invoke: Invoke) {
        val out = JSObject()
            .put("distributors", UnifiedPush.getDistributors(activity).size)
            .put("notices", NotificationManagerCompat.from(activity).areNotificationsEnabled())
        Push.endpoint(activity)?.let { (url, key, auth) ->
            out.put("endpoint", url).put("p256dh", key).put("auth", auth)
        }
        invoke.resolve(out)
    }

    companion object {
        /** A coordinate as exactly 32 bytes, big-endian. */
        fun fixed32(n: BigInteger): ByteArray {
            val b = n.toByteArray()
            return when {
                b.size == 32 -> b
                b.size > 32 -> b.copyOfRange(b.size - 32, b.size)
                else -> ByteArray(32 - b.size) + b
            }
        }

        /** A DER ECDSA signature as r and s (64 bytes), lower-case hex: the form the hub takes. */
        fun raw(der: ByteArray): String {
            // SEQUENCE { INTEGER r, INTEGER s }, short lengths for P-256.
            var i = 2
            if (der[1].toInt() and 0x80 != 0) i += der[1].toInt() and 0x7f
            fun int(): BigInteger {
                check(der[i].toInt() == 0x02)
                val len = der[i + 1].toInt()
                val v = BigInteger(1, der.copyOfRange(i + 2, i + 2 + len))
                i += 2 + len
                return v
            }
            val r = int()
            val s = int()
            return (fixed32(r) + fixed32(s)).joinToString("") { "%02x".format(it) }
        }
    }
}

/** What the push side keeps: the endpoint of the distributor and the words of the notice. App-private, no backup. */
object Push {
    private const val FILE = "appkit-push"

    private fun prefs(context: Context) = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    fun saveNotice(context: Context, title: String, text: String) {
        prefs(context).edit().putString("title", title).putString("text", text).apply()
    }

    fun saveEndpoint(context: Context, url: String, key: String, auth: String) {
        prefs(context).edit().putString("url", url).putString("key", key).putString("auth", auth).apply()
    }

    fun endpoint(context: Context): Triple<String, String, String>? {
        val p = prefs(context)
        val url = p.getString("url", null) ?: return null
        val key = p.getString("key", null) ?: return null
        val auth = p.getString("auth", null) ?: return null
        return Triple(url, key, auth)
    }

    fun forget(context: Context) {
        prefs(context).edit().clear().apply()
    }

    /** The notice of a message: the words the app gave, never what the message holds (PWA-06). */
    fun notify(context: Context) {
        val p = prefs(context)
        val title = p.getString("title", null) ?: return
        val text = p.getString("text", "") ?: ""
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel("hub", title, NotificationManager.IMPORTANCE_HIGH)
            )
        }
        val open = context.packageManager.getLaunchIntentForPackage(context.packageName)
        val tap = open?.let {
            PendingIntent.getActivity(
                context,
                0,
                it,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        }
        val notice = NotificationCompat.Builder(context, "hub")
            .setSmallIcon(context.applicationInfo.icon)
            .setContentTitle(title)
            .setContentText(text)
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
        tap?.let { notice.setContentIntent(it) }
        if (NotificationManagerCompat.from(context).areNotificationsEnabled()) {
            try {
                NotificationManagerCompat.from(context).notify(1, notice.build())
            } catch (e: SecurityException) {
            }
        }
    }
}
