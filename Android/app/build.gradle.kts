plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    // Uppercase namespace/applicationId is intentional (matches the other platform app IDs).
    namespace = "com.tsubuzaki.WoooshGo"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.tsubuzaki.WoooshGo"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1"
    }

    buildTypes {
        debug {
            isDebuggable = true
        }
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }
    buildFeatures {
        compose = true
    }
    lint {
        // Keep the intentionally uppercase package/applicationId out of lint noise.
        disable += listOf("PackageName", "PackageNaming")
    }
    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    implementation(libs.kotlinx.coroutines.android)

    implementation(libs.core.ktx)
    implementation(libs.lifecycle.runtime.ktx)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.lifecycle.viewmodel.compose)

    implementation(libs.datastore.preferences)

    // UniFFI bindings for wooosh-core (generated Kotlin in src/main/java/uniffi/,
    // libwooosh_core.so in src/main/jniLibs/) call the library through JNA.
    implementation(variantOf(libs.jna) { artifactType("aar") })

    implementation(libs.zxing.core)
    implementation(libs.zxing.android.embedded)

    implementation(libs.activity.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.material.icons.extended)
    implementation(libs.material3)
}
