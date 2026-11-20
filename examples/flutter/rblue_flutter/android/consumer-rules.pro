# Matches Java_com_rblue_RblueNative_nativeInit in librblue_c.so.
-keep class com.rblue.RblueNative { *; }
-keep class com.rblue.AdapterStateMonitor { *; }

# The vendored btleplug Java helpers are resolved by name over JNI and some are
# instantiated reflectively. Stripping or renaming them breaks initialization.
-keep class com.nonpolynomial.btleplug.android.impl.** { *; }
-keep class io.github.gedgygedgy.rust.** { *; }
