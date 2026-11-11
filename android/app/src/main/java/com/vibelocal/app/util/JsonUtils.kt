package com.vibelocal.app.util

import org.json.JSONArray
import org.json.JSONObject

/** Small org.json helpers used across the data layer. All reads are defensive. */

fun JSONObject.str(key: String, fallback: String = ""): String = optString(key, fallback)

fun JSONObject.strOrNull(key: String): String? =
    if (isNull(key)) null else optString(key).takeIf { it.isNotEmpty() }

fun JSONObject.double(key: String, fallback: Double = 0.0): Double = optDouble(key, fallback)

fun JSONObject.doubleOrNull(key: String): Double? =
    if (isNull(key)) null else optDouble(key).takeIf { !it.isNaN() }

fun JSONObject.long(key: String, fallback: Long = 0L): Long = optLong(key, fallback)

fun JSONObject.int(key: String, fallback: Int = 0): Int = optInt(key, fallback)

fun JSONObject.bool(key: String, fallback: Boolean = false): Boolean = optBoolean(key, fallback)

fun JSONObject.stringList(key: String): List<String> {
    val arr = optJSONArray(key) ?: return emptyList()
    return (0 until arr.length()).mapNotNull { i ->
        arr.optString(i).takeIf { it.isNotEmpty() }
    }
}

fun JSONObject.doubleMap(key: String): Map<String, Double> {
    val obj = optJSONObject(key) ?: return emptyMap()
    val out = LinkedHashMap<String, Double>()
    for (k in obj.keys()) {
        val v = obj.optDouble(k, Double.NaN)
        if (!v.isNaN()) out[k] = v
    }
    return out
}

fun JSONObject.stringMap(key: String): Map<String, String> {
    val obj = optJSONObject(key) ?: return emptyMap()
    val out = LinkedHashMap<String, String>()
    for (k in obj.keys()) {
        val v = obj.optString(k)
        if (v.isNotEmpty()) out[k] = v
    }
    return out
}

fun JSONArray.toObjectList(): List<JSONObject> =
    (0 until length()).mapNotNull { i -> optJSONObject(i) }

/** First non-null string among [keys], for tolerant parsing of evolving schemas. */
fun JSONObject.firstString(vararg keys: String): String? {
    for (k in keys) {
        val v = strOrNull(k)
        if (v != null) return v
    }
    return null
}

fun JSONObject.firstDouble(vararg keys: String): Double? {
    for (k in keys) {
        val v = doubleOrNull(k)
        if (v != null) return v
    }
    return null
}

/** Format bytes as a human readable size without hardcoding any device data. */
fun formatBytes(bytes: Long): String {
    if (bytes <= 0) return "0 B"
    val units = arrayOf("B", "KB", "MB", "GB", "TB")
    var value = bytes.toDouble()
    var unit = 0
    while (value >= 1024.0 && unit < units.lastIndex) {
        value /= 1024.0
        unit++
    }
    return if (unit == 0) "${bytes} B" else String.format("%.2f %s", value, units[unit])
}