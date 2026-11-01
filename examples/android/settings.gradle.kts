// Gradle root for the rblue Android example. The build owns two modules that
// live outside this directory (the UniFFI AAR project), pulled in by explicit
// projectDir mapping - see examples/README.md for the non-interference
// contract that makes this layout safe.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "rblue-android-example"

include(":app")
include(":rblue")
project(":rblue").projectDir = file("../../bindings/kotlin/rblue")
include(":btleplug-java")
project(":btleplug-java").projectDir = file("../../bindings/kotlin/btleplug-java")
