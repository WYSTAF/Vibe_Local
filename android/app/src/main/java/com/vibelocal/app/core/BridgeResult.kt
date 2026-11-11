package com.vibelocal.app.core

/**
 * Result envelope for every native bridge call.
 *
 * [Ok.data] holds the parsed `data` field of the native JSON envelope and may
 * be an [org.json.JSONObject], [org.json.JSONArray], String, Number, Boolean
 * or null depending on the endpoint.
 */
sealed class BridgeResult {

    data class Ok(val data: Any?) : BridgeResult()

    data class Err(val message: String) : BridgeResult()

    val isOk: Boolean get() = this is Ok

    /** Convenience: typed data or null. */
    inline fun <reified T> dataAs(): T? = (this as? Ok)?.data as? T

    companion object {
        fun err(message: String?): Err = Err(message ?: "Unknown native error")
    }
}