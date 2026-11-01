plugins {
    alias(libs.plugins.android.library)
}

android {
    // The namespace is the fork's library identity, not the Java package
    // roots inside src/main/java (two packages are vendored there).
    namespace = "com.nonpolynomial.btleplug.android"
    compileSdk = 35

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }

    // Vendored third-party code; the keep rules in consumer-rules.pro are
    // what matters, lint findings on the fork's own style stay out of the
    // example's build.
    lint {
        abortOnError = false
    }
}
