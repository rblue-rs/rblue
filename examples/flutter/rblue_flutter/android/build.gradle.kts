group = "dev.rblue.rblue_flutter"
version = "1.0-SNAPSHOT"

buildscript {
    val kotlinVersion = "2.2.20"
    repositories {
        google()
        mavenCentral()
    }

    dependencies {
        classpath("com.android.tools.build:gradle:8.11.1")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:$kotlinVersion")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

plugins {
    id("com.android.library")
    id("kotlin-android")
}

// The prebuilt C ABI libraries live in src/main/jniLibs/<abi>/librblue_c.so;
// they are build artifacts, refreshed by examples/flutter/tools/build-libs.sh.
val rblueAbis = listOf("arm64-v8a", "armeabi-v7a", "x86", "x86_64")

val checkRblueLibs by tasks.registering {
    doFirst {
        val missing = rblueAbis.filterNot { abi ->
            file("src/main/jniLibs/$abi/librblue_c.so").exists()
        }
        if (missing.isNotEmpty()) {
            throw GradleException(
                "rblue_flutter: missing librblue_c.so for $missing. " +
                    "Run examples/flutter/tools/build-libs.sh to cross-build them.",
            )
        }
    }
}

tasks.named("preBuild") { dependsOn(checkRblueLibs) }

android {
    namespace = "dev.rblue.rblue_flutter"

    compileSdk = 36

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = JavaVersion.VERSION_17.toString()
    }

    sourceSets {
        getByName("main") {
            java.srcDirs("src/main/kotlin")
            // Rust resolves these shared helper classes by name through JNI.
            java.srcDir("../../../../bindings/kotlin/btleplug-java/src/main/java")
        }
    }

    defaultConfig {
        // rblue-android requires API 24+.
        minSdk = 24
        consumerProguardFiles(
            "consumer-rules.pro",
            "../../../../bindings/kotlin/btleplug-java/consumer-rules.pro",
        )
    }
}
