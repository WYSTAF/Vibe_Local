# Vibe Local ProGuard rules.

# --- JNI bridge -------------------------------------------------------------
# The native library resolves these symbols by name; never rename or strip them.
-keep class com.vibelocal.app.core.VibeLocalBridge {
    external <methods>;
    public static ** INSTANCE;
}
-keepclasseswithmembernames class com.vibelocal.app.core.VibeLocalBridge {
    native <methods>;
}

# --- org.json ---------------------------------------------------------------
# org.json is part of the Android platform; keep reflective access quiet.
-dontwarn org.json.**

# --- Kotlin coroutines ------------------------------------------------------
-dontwarn kotlinx.coroutines.**
-keepclassmembers class kotlinx.coroutines.** {
    volatile <fields>;
}

# --- Compose / AndroidX -----------------------------------------------------
# Standard AndroidX + Compose consumer rules ship with the libraries.
-dontwarn androidx.**

# --- Model classes parsed via org.json --------------------------------------
# Parsed manually (no reflection), but keep names for readable crash reports.
-keep class com.vibelocal.app.core.** { *; }

# Keep line numbers for useful stack traces.
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile