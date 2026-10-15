import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
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
    implementation(project(":tauri-android"))
    testImplementation("junit:junit:4.13.2")
    testImplementation("androidx.test:core:1.6.1")
    testImplementation("org.robolectric:robolectric:4.16")
}
