import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "io.github.wkdsteve.appkit"
    compileSdk = 36

    defaultConfig {
        // Android 7: what Tauri's own Android project asks; the trigger key with the device credential needs 11 and
        // falls back to biometrics only below that.
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

// Every dependency with its reason (MREL-04); exact versions, no ranges (AND-12).
dependencies {
    // The plugin base of Tauri and the activity it runs in (AppCompat, so a fragment can show the prompt).
    implementation("androidx.core:core-ktx:1.9.0")
    implementation("androidx.appcompat:appcompat:1.6.0")
    // BiometricPrompt with a CryptoObject: the trigger key signs only after the biometrics or the device lock (AND-05).
    implementation("androidx.biometric:biometric:1.1.0")
    // The QR code of a pairing: ZXing, Apache-2.0, no network and no analytics (ML Kit of Google would report usage).
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    // Push without Firebase: the UnifiedPush connector speaks to a distributor app on the phone and decrypts Web Push.
    implementation("org.unifiedpush.android:connector:3.3.5")
    implementation(project(":tauri-android"))
}
