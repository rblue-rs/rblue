# The fork's Java helpers are resolved by name over JNI (Rust side:
# env.find_class / register natives) and instantiated reflectively; stripping
# or renaming them breaks the Android backend at init with Error::Backend.
-keep class com.nonpolynomial.btleplug.android.impl.** { *; }
-keep class io.github.gedgygedgy.rust.** { *; }
