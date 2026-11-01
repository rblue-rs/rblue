plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "com.rblue.example"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.rblue.example"
        minSdk = 24
        targetSdk = 35
        versionCode = providers.gradleProperty("rblueVersionCode")
            .orElse(System.getenv("RBLUE_VERSION_CODE") ?: "1")
            .get()
            .toInt()
        versionName = providers.gradleProperty("rblueVersionName")
            .orElse(System.getenv("RBLUE_VERSION_NAME") ?: "0.1.0")
            .get()
    }

    // Matches the `backend` dimension of :rblue: `real` ships the JNI
    // Android backend (devices only), `mock` the mock backend (emulators,
    // CI smoke).
    flavorDimensions += "backend"
    productFlavors {
        create("real") { dimension = "backend" }
        create("mock") { dimension = "backend" }
    }

    buildTypes {
        release {
            // Exercise the consumer-rules.pro keep rules of the AAR on every
            // release build (M-A3): R8 must not touch the generated bindings,
            // the JNI entry point, or the fork's reflective Java helpers.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    buildFeatures {
        compose = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    testOptions {
        // Pure-JVM unit tests (JSON contract, backoff math, error mapping).
        unitTests.isIncludeAndroidResources = false
    }
}

dependencies {
    implementation(project(":rblue"))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.kotlinx.serialization.json)
    debugImplementation(libs.androidx.compose.ui.tooling)

    testImplementation(libs.junit)
}
