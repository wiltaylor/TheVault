import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

extensions.configure<com.android.build.api.dsl.LibraryExtension> {
    namespace = "dev.wiltaylor.thevault.device"
    compileSdk = 36

    defaultConfig {
        minSdk = 33
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
        unitTests.all { test ->
            // The JNI round-trip test loads a host build of the plugin's Rust library
            // (`just android-test` builds it), since the app's library is built for the phone.
            System.getenv("VAULT_CORE_LIBRARY")?.let { test.systemProperty("vault.core.library", it) }
            // Robolectric downloads its Android jars under the home folder, which the dev
            // container's user lacks (Java reports `?`); the Gradle home is a cached volume there.
            val robolectricHome = File(gradle.gradleUserHomeDir, "robolectric-home")
            test.systemProperty("user.home", robolectricHome.path)
            test.doFirst { robolectricHome.mkdirs() }
            // The Rust core is loaded once per JVM, into one class loader: plain JUnit and each
            // Robolectric sandbox load it separately, so every test class gets a JVM of its own.
            test.forkEvery = 1
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.activity:activity-ktx:1.10.1")
    // Tauri's plugin lifecycle hands the activity over as an AppCompatActivity.
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.work:work-runtime-ktx:2.10.5")
    // E.164 numbers in device event payloads (D24).
    implementation("com.googlecode.libphonenumber:libphonenumber:8.13.52")
    // QR scanning without Google Play services (D10).
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    // UnifiedPush (D4): registration, and WebPush decryption with keys it keeps.
    implementation("org.unifiedpush.android:connector:3.3.5")
    implementation(project(":tauri-android"))
    testImplementation("junit:junit:4.13.2")
    testImplementation("androidx.test:core:1.6.1")
    testImplementation("org.robolectric:robolectric:4.16")
    testImplementation("androidx.work:work-testing:2.10.5")
    testImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
    // The connector's WebPush decrypter, compiled against in the interop test.
    testImplementation("com.google.crypto.tink:tink:1.23.0")
}
