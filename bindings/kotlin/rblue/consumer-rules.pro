# Consumer rules for apps that use the rblue AAR with R8/ProGuard.
#
# Everything below is resolved through JNI or reflection by name at runtime;
# minifying any of it breaks the backend with Error::Backend /
# UnsatisfiedLinkError instead of a clean startup.

# JNI entry point declared in Kotlin, implemented by the exported symbol
# Java_com_rblue_RblueNative_nativeInit in librblue_ffi.so.
-keep class com.rblue.RblueNative { *; }
-keep class com.rblue.AdapterStateMonitor { *; }

# The btleplug fork's helper classes: instantiated and registered over JNI by
# name (com.nonpolynomial.btleplug.android.impl.*) and through the gedgygedgy
# future/stream support classes the fork's Rust side drives reflectively.
-keep class com.nonpolynomial.btleplug.android.impl.** { *; }
-keep class io.github.gedgygedgy.rust.** { *; }

# UniFFI generated bindings: JNA structures, callback proxies, and the
# UniffiLib entry table must keep their names/layout.
-keep class uniffi.rblue_ffi.** { *; }

# JNA plumbing (the JNA jar ships its own rules; these are defensive keeps for
# the surface the generated code actually touches).
-keep class com.sun.jna.** { *; }
-keepclassmembers class * implements com.sun.jna.Callback { *; }
# JNA ships optional desktop AWT helpers in the same artifact; Android has no
# java.awt and these paths are unreachable.
-dontwarn java.awt.Component
-dontwarn java.awt.GraphicsEnvironment
-dontwarn java.awt.HeadlessException
-dontwarn java.awt.Window

-keepattributes Signature, InnerClasses, EnclosingMethod, *Annotation*
