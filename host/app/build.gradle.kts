plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "dev.r1os.host"
    compileSdk {
        version = release(36)
    }

    defaultConfig {
        applicationId = "dev.r1os.host"
        // The R1 runs Android 14 (API 34). targetSdk 34 also keeps API 35's forced edge-to-edge off.
        minSdk = 30
        targetSdk = 34
        // 2: the native shell. The image's copy must not be older than an adb-installed one, or
        // Android keeps the installed one over a newly flashed image.
        // 3: signed with r1os's platform key (T-133). Higher than the debug-signed copies already
        // installed, so the image's copy replaces them (Android keeps the host's data).
        versionCode = 3
        versionName = "0.2.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Gradle signs with the debug key; tools/build-host.sh then re-signs out/r1os-host.apk
            // with r1os's platform key (T-133), the key the image is signed with.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    // The wardrobe (agents' forms, eyes and what they wear) is one file the shell and r1d read too (T-062):
    // packed as a Java resource, so ui/Wardrobe.kt reads it without a Context.
    sourceSets {
        getByName("main") {
            resources.srcDir("../../shell/src/ui/wardrobe")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
