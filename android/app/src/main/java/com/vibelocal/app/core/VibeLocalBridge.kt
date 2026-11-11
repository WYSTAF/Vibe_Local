package com.vibelocal.app.core

/**
 * Raw JNI surface of the Vibe Local engine.
 *
 * Contract (must match the Rust `vibe-local-jni` crate exactly):
 * every function returns a JSON string of shape
 *   {"ok":true,"data":...}   or   {"ok":false,"error":"..."}
 *
 * These are instance methods on the Kotlin `object` singleton (INSTANCE);
 * the Rust side binds them via JNI RegisterNatives or the
 * Java_com_vibelocal_app_core_VibeLocalBridge_<name> symbol convention.
 *
 * NEVER call these directly from UI code — go through [SafeBridge], which
 * catches UnsatisfiedLinkError / JSON errors and wraps results in [BridgeResult].
 */
object VibeLocalBridge {

    init {
        // Trigger the one-time System.loadLibrary attempt. Failure is tolerated:
        // SafeBridge checks NativeLib.isAvailable before every call.
        NativeLib.ensureLoaded()
    }

    external fun scanHardware(): String

    external fun recommend(): String

    external fun inspectGguf(path: String): String

    external fun runVibeMark(path: String, budgetSecs: Int): String

    external fun leaderboard(profile: String): String

    external fun submitLeaderboard(entryJson: String): String

    external fun startServer(port: Int, lan: Boolean, token: String, dataDir: String): String

    external fun stopServer(): String

    external fun serverStatus(): String

    external fun chat(prompt: String, stripThink: Boolean): String

    external fun unloadModel(): String

    external fun setThermalReading(celsius: Double): String

    external fun setIdleTimeout(seconds: Int): String
}