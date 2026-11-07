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

// Prebuilt rblue-c libraries are part of the published package. Their exact
// source, build command, checksums, and license inventory are documented in
// native/BUILDING.md and THIRD_PARTY_NOTICES.md.
val rblueAbis = listOf("arm64-v8a", "armeabi-v7a", "x86", "x86_64")

val checkRblueLibs by tasks.registering {
    doFirst {
        val missing = rblueAbis.filterNot { abi ->
            file("src/main/jniLibs/$abi/librblue_c.so").exists()
        }
        if (missing.isNotEmpty()) {
            throw GradleException(
                "rblue_flutter: published package is missing librblue_c.so " +
                    "for $missing; reinstall the package or report a packaging bug.",
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
            java.srcDirs("src/main/kotlin", "src/main/java")
        }
    }

    defaultConfig {
        // rblue-android requires API 24+.
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }
}
