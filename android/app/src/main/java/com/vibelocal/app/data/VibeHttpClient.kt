package com.vibelocal.app.data

import com.vibelocal.app.core.HardwareInfo
import com.vibelocal.app.core.LeaderboardData
import com.vibelocal.app.core.Recommendations
import com.vibelocal.app.core.ServerStatus
import com.vibelocal.app.core.VibeMarkResult
import com.vibelocal.app.core.extractChatContent
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

/**
 * Minimal HTTP client for the Vibe-Link API using HttpURLConnection + org.json
 * only (no third-party network libraries).
 *
 * Endpoints (native server on 127.0.0.1:20129, or a remote LAN host):
 *   GET  /api/hardware, /api/recommendations, /api/leaderboard?profile=, /api/status
 *   POST /api/load, /api/unload, /api/vibemark
 *   POST /v1/chat/completions (OpenAI shape + "strip_think")
 *   GET  /v1/models
 *
 * When [token] is set, every request carries `X-Vibe-Local-Token` as required
 * by LAN mode; mutating POSTs always send `Content-Type: application/json`.
 */
class VibeHttpClient {

    @Volatile
    var token: String? = null

    // ------------------------------------------------------------------ core

    suspend fun getJson(base: String, path: String): Result<Any> =
        withContext(Dispatchers.IO) { request(base, path, "GET", null) }

    suspend fun postJson(base: String, path: String, body: JSONObject?): Result<Any> =
        withContext(Dispatchers.IO) { request(base, path, "POST", body) }

    private fun request(base: String, path: String, method: String, body: JSONObject?): Result<Any> {
        var conn: HttpURLConnection? = null
        return try {
            val url = URL(base.trimEnd('/') + path)
            conn = url.openConnection() as HttpURLConnection
            conn.connectTimeout = CONNECT_TIMEOUT_MS
            // Benchmarks and chat can legitimately take a long time.
            conn.readTimeout =
                if (path.startsWith("/api/vibemark") || path.startsWith("/v1/chat")) LONG_READ_TIMEOUT_MS
                else READ_TIMEOUT_MS
            conn.requestMethod = method
            conn.setRequestProperty("Accept", "application/json")
            token?.takeIf { it.isNotBlank() }?.let {
                conn.setRequestProperty("X-Vibe-Local-Token", it)
            }
            if (method == "POST") {
                conn.doOutput = true
                conn.setRequestProperty("Content-Type", "application/json")
                val bytes = (body ?: JSONObject()).toString().toByteArray(Charsets.UTF_8)
                conn.setFixedLengthStreamingMode(bytes.size)
                conn.outputStream.use { it.write(bytes) }
            }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val text = stream?.bufferedReader(Charsets.UTF_8)?.use { it.readText() } ?: ""
            if (code !in 200..299) {
                return Result.failure(IOException("HTTP $code: ${extractErrorMessage(text)}"))
            }
            val trimmed = text.trim()
            if (trimmed.isEmpty()) return Result.success(JSONObject())
            val parsed: Any =
                if (trimmed.startsWith("[")) JSONArray(trimmed) else JSONObject(trimmed)
            Result.success(parsed)
        } catch (e: Exception) {
            Result.failure(e)
        } finally {
            conn?.disconnect()
        }
    }

    private fun extractErrorMessage(body: String): String {
        return try {
            val obj = JSONObject(body)
            obj.optString("error").takeIf { it.isNotBlank() }
                ?: obj.optString("message").takeIf { it.isNotBlank() }
                ?: body.take(200)
        } catch (e: Exception) {
            body.take(200).ifBlank { "empty body" }
        }
    }

    // ---------------------------------------------------------------- typed

    suspend fun hardware(base: String): Result<HardwareInfo> =
        getJson(base, "/api/hardware").mapCatching { HardwareInfo.fromJson(it as JSONObject) }

    suspend fun recommendations(base: String): Result<Recommendations> =
        getJson(base, "/api/recommendations").mapCatching { Recommendations.fromJson(it as JSONObject) }

    suspend fun leaderboard(base: String, profile: String): Result<LeaderboardData> {
        val encoded = URLEncoder.encode(profile, "UTF-8")
        return getJson(base, "/api/leaderboard?profile=$encoded")
            .mapCatching { LeaderboardData.fromJson(it as JSONObject) }
    }

    suspend fun status(base: String): Result<ServerStatus> =
        getJson(base, "/api/status").mapCatching { ServerStatus.fromJson(it as JSONObject) }

    suspend fun load(base: String, path: String, backend: String?, ngl: Int, ctx: Int): Result<Unit> {
        val body = JSONObject()
            .put("path", path)
            .put("ngl", ngl)
            .put("ctx", ctx)
        if (!backend.isNullOrBlank()) body.put("backend", backend)
        return postJson(base, "/api/load", body).map { }
    }

    suspend fun unload(base: String): Result<Unit> =
        postJson(base, "/api/unload", null).map { }

    suspend fun vibemark(base: String, path: String, budgetSecs: Int): Result<VibeMarkResult> {
        val body = JSONObject().put("path", path).put("budget_secs", budgetSecs)
        return postJson(base, "/api/vibemark", body)
            .mapCatching { VibeMarkResult.fromJson(it as JSONObject) }
    }

    suspend fun chat(
        base: String,
        messages: JSONArray,
        model: String?,
        stripThink: Boolean,
    ): Result<String> {
        val body = JSONObject()
            .put("messages", messages)
            .put("stream", false)
            .put("strip_think", stripThink)
        if (!model.isNullOrBlank()) body.put("model", model)
        return postJson(base, "/v1/chat/completions", body).mapCatching {
            extractChatContent(it) ?: throw JSONException("No assistant content in response")
        }
    }

    suspend fun models(base: String): Result<List<String>> =
        getJson(base, "/v1/models").mapCatching { resp ->
            val arr = (resp as JSONObject).optJSONArray("data") ?: JSONArray()
            (0 until arr.length()).mapNotNull { i ->
                arr.optJSONObject(i)?.optString("id")?.takeIf { it.isNotBlank() }
            }
        }

    companion object {
        private const val CONNECT_TIMEOUT_MS = 10_000
        private const val READ_TIMEOUT_MS = 30_000
        private const val LONG_READ_TIMEOUT_MS = 10 * 60_000
    }
}