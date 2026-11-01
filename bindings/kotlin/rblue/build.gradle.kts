plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "com.rblue"
    compileSdk = 35

    sourceSets {
        getByName("main") {
            // The canonical uniffi-bindgen output (bindings/kotlin/generated,
            // committed for review) is the single copy of the generated
            // bindings; compiling it here keeps the AAR drift-free against
            // the reviewed source. CI rejects any drift the script produced.
            kotlin.srcDir("../generated")
        }
    }

    // The `backend` dimension mirrors the two .so pipelines of
    // scripts/build-bindings.sh: `real` links the JNI Android backend,
    // `mock` links the mock backend (no Bluetooth hardware, runs on
    // emulators). Flavors only switch the jniLibs source sets and the
    // RblueAndroid.initialize implementation; the UniFFI surface is
    // identical in both.
    flavorDimensions += "backend"
    productFlavors {
        create("real") { dimension = "backend" }
        create("mock") { dimension = "backend" }
    }

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    // uniffi-generated Kotlin talks to Rust through JNA, and its suspend
    // methods are built on kotlinx.coroutines; both are part of the public
    // surface of the generated file, hence `api`.
    api(libs.jna)
    api(libs.kotlinx.coroutines.core)
    // The fork's helper classes are resolved by name over JNI at runtime;
    // they must survive into the consuming app, so they are an `api`
    // dependency (their consumer-rules.pro keep rules propagate too).
    api(project(":btleplug-java"))
}
