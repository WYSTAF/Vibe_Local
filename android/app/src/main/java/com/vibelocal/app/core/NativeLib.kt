package com.vibelocal.app.core

import android.util.Log

/**
 * Loads libvibe_local_jni.so exactly once and remembers whether it succeeded.
 *
 * When loading fails (library not bundled for this ABI, missing dependencies,
 * linker error, ...) the app keeps working in HTTP fallback mode against a
 * user-entered Vibe-Link host. Nothing in the app may crash because of a
 * missing native library.
 */
object NativeLib {

    private const val TAG = "VibeLocalNative"
    const val LIB_NAME = "vibe_local_jni"

    /** True when System.loadLibrary succeeded and JNI calls are safe to make. */
    val isAvailable: Boolean

    /** Human-readable reason when [isAvailable] is false; null otherwise. */
    val loadError: String?

    init {
        var error: String? = null
        try {
            System.loadLibrary(LIB_NAME)
            Log.i(TAG, "Loaded native library $LIB_NAME")
        } catch (t: Throwable) {
            // UnsatisfiedLinkError is the common case; be paranoid about anything else.
            error = t.message ?: t.javaClass.simpleName
            Log.w(TAG, "Native library $LIB_NAME unavailable: $error", t)
        }
        isAvailable = error == null
        loadError = error
    }

    /** Touch this object to force the load attempt (used by [VibeLocalBridge]). */
    fun ensureLoaded(): Boolean = isAvailable
}