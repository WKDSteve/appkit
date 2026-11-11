package io.github.wkdsteve.appkit

import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * The messages of the UnifiedPush distributor. The endpoint is kept for the shell, which subscribes it at the hub; a
 * message only shows the notice the app named. Its content (the kind of event, at most an id) is not shown: the app
 * reads the rest over its paired way when the person opens it.
 */
class PushReceiver : PushService() {
    override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
        val keys = endpoint.pubKeySet ?: return
        Push.saveEndpoint(this, endpoint.url, keys.pubKey, keys.auth)
    }

    override fun onMessage(message: PushMessage, instance: String) {
        // Only a message the connector could decrypt is from the hub (Web Push with the keys of this app).
        if (message.decrypted) Push.notify(this)
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        Push.forget(this)
    }

    override fun onUnregistered(instance: String) {
        Push.forget(this)
    }
}
