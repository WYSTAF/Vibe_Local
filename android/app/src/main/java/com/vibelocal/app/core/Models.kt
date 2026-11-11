package com.vibelocal.app.core

import com.vibelocal.app.util.bool
import com.vibelocal.app.util.double
import com.vibelocal.app.util.doubleMap
import com.vibelocal.app.util.firstDouble
import com.vibelocal.app.util.firstString
import com.vibelocal.app.util.int
import com.vibelocal.app.util.long
import com.vibelocal.app.util.str
import com.vibelocal.app.util.stringList
import com.vibelocal.app.util.stringMap
import com.vibelocal.app.util.strOrNull
import com.vibelocal.app.util.toObjectList
import org.json.JSONArray
import org.json.JSONObject

/**
 * Typed mirrors of the Vibe-Link HTTP/JNI payloads. Every value originates
 * from the engine (bridge or HTTP) — nothing here invents hardware numbers.
 * Parsing is defensive: unknown fields are ignored, missing fields get
 * neutral defaults so schema evolution cannot crash the UI.
 */

data class GpuInfo(
    val name: String,
    val vendor: String,
    val vramMb: Long,
    val backend: String,
) {
    companion object {
        fun fromJson(o: JSONObject): GpuInfo = GpuInfo(
            name = o.str("name", "Unknown GPU"),
            vendor = o.str("vendor"),
            vramMb = o.long("vram_mb"),
            backend = o.str("backend"),
        )
    }
}

data class PortableInfo(
    val available: Boolean,
    val root: String?,
) {
    companion object {
        fun fromJson(o: JSONObject): PortableInfo = PortableInfo(
            available = o.bool("available"),
            root = o.strOrNull("root"),
        )
    }
}

data class HardwareInfo(
    val os: String,
    val arch: String,
    val cpuModel: String,
    val cpuCores: Int,
    val cpuFlags: List<String>,
    val ramTotalMb: Long,
    val ramFreeMb: Long,
    val gpus: List<GpuInfo>,
    val preferredBackend: String,
    val tier: String,
    val portable: PortableInfo?,
) {
    companion object {
        fun fromJson(o: JSONObject): HardwareInfo = HardwareInfo(
            os = o.str("os"),
            arch = o.str("arch"),
            cpuModel = o.str("cpu_model"),
            cpuCores = o.int("cpu_cores"),
            cpuFlags = o.stringList("cpu_flags"),
            ramTotalMb = o.long("ram_total_mb"),
            ramFreeMb = o.long("ram_free_mb"),
            gpus = o.optJSONArray("gpus")?.toObjectList()?.map(GpuInfo::fromJson) ?: emptyList(),
            preferredBackend = o.str("preferred_backend"),
            tier = o.str("tier"),
            portable = o.optJSONObject("portable")?.let(PortableInfo::fromJson),
        )
    }
}

data class ModelCandidate(
    val modelId: String,
    val hfRepo: String,
    val filename: String,
    val quant: String,
    /** "moe" or "dense". */
    val family: String,
    val totalParamsB: Double,
    val activeParamsB: Double,
    val sizeBytes: Long,
    val downloadUrl: String,
    val estRamMb: Long,
    /** "vram" or "ram". */
    val fit: String,
    val ngl: Int,
    val reasons: List<String>,
    val estSpeedupVsDense7b: Double,
) {
    val isMoe: Boolean get() = family.equals("moe", ignoreCase = true)

    /** Only Hugging Face HTTPS URLs are ever handed to DownloadManager. */
    val isDownloadUrlTrusted: Boolean
        get() = downloadUrl.startsWith("https://huggingface.co/")

    companion object {
        fun fromJson(o: JSONObject): ModelCandidate = ModelCandidate(
            modelId = o.str("model_id"),
            hfRepo = o.str("hf_repo"),
            filename = o.str("filename"),
            quant = o.str("quant"),
            family = o.str("family", "dense"),
            totalParamsB = o.double("total_params_b"),
            activeParamsB = o.double("active_params_b"),
            sizeBytes = o.long("size_bytes"),
            downloadUrl = o.str("download_url"),
            estRamMb = o.long("est_ram_mb"),
            fit = o.str("fit", "ram"),
            ngl = o.int("ngl"),
            reasons = o.stringList("reasons"),
            estSpeedupVsDense7b = o.double("est_speedup_vs_dense_7b"),
        )
    }
}

data class Recommendations(
    val tier: String,
    val rationale: String,
    val candidates: List<ModelCandidate>,
) {
    val prefersMoe: Boolean get() = tier.contains("moe", ignoreCase = true)

    companion object {
        fun fromJson(o: JSONObject): Recommendations = Recommendations(
            tier = o.str("tier"),
            rationale = o.str("rationale"),
            candidates = o.optJSONArray("candidates")?.toObjectList()
                ?.map(ModelCandidate::fromJson) ?: emptyList(),
        )
    }
}

data class LeaderboardEntry(
    val id: String,
    val modelName: String,
    val modelId: String,
    val hfRepo: String?,
    val downloadUrl: String?,
    val quant: String?,
    val family: String?,
    val backend: String?,
    val device: String?,
    val profile: String?,
    val totalParamsB: Double,
    val activeParamsB: Double,
    val vibeQualityScore: Double,
    val powerPerParameter: Double,
    val paretoOptimal: Boolean,
    val pillarScores: Map<String, Double>,
    val provenance: String?,
) {
    companion object {
        fun fromJson(o: JSONObject): LeaderboardEntry {
            val modelId = o.firstString("model_id", "model", "id") ?: ""
            // Per-pillar scores may arrive as "pillar_scores" or "pillars" object.
            val pillars = o.doubleMap("pillar_scores").ifEmpty { o.doubleMap("pillars") }
            return LeaderboardEntry(
                id = o.firstString("id")
                    ?: (modelId + "|" + o.str("profile") + "|" + o.str("backend")),
                modelName = o.firstString("model_name", "name", "model", "model_id") ?: modelId,
                modelId = modelId,
                hfRepo = o.strOrNull("hf_repo"),
                downloadUrl = o.firstString("download_url", "hf_url"),
                quant = o.strOrNull("quant"),
                family = o.strOrNull("family"),
                backend = o.strOrNull("backend"),
                device = o.strOrNull("device"),
                profile = o.strOrNull("profile"),
                totalParamsB = o.firstDouble("total_params_b") ?: 0.0,
                activeParamsB = o.firstDouble("active_params_b") ?: 0.0,
                vibeQualityScore = o.firstDouble("vibe_quality_score", "quality_score") ?: 0.0,
                powerPerParameter = o.firstDouble("power_per_parameter") ?: 0.0,
                paretoOptimal = o.bool("pareto_optimal"),
                pillarScores = pillars,
                provenance = o.strOrNull("provenance"),
            )
        }
    }
}

data class LeaderboardData(
    val pillars: List<String>,
    val pillarLabels: Map<String, String>,
    val profiles: Map<String, String>,
    val entries: List<LeaderboardEntry>,
) {
    companion object {
        /** The canonical 10 pillar ids, used when the payload omits the list. */
        val DEFAULT_PILLARS = listOf(
            "code_generation",
            "systems_polyglot",
            "math_reasoning",
            "hard_science_gpqa",
            "instruction_schema",
            "long_context_needle",
            "truthfulness_anti_sycophancy",
            "quant_retention_kl",
            "hardware_throughput",
            "latency_thermal_efficiency",
        )

        fun fromJson(o: JSONObject): LeaderboardData {
            val pillars = o.stringList("pillars").ifEmpty { DEFAULT_PILLARS }
            // profiles may be {id: "label"} or {id: {"label": ...}}.
            val profilesObj = o.optJSONObject("profiles")
            val profiles = LinkedHashMap<String, String>()
            if (profilesObj != null) {
                for (k in profilesObj.keys()) {
                    val v = profilesObj.opt(k)
                    profiles[k] = when (v) {
                        is String -> v
                        is JSONObject -> v.firstString("label", "name") ?: k
                        else -> k
                    }
                }
            }
            return LeaderboardData(
                pillars = pillars,
                pillarLabels = o.stringMap("pillar_labels"),
                profiles = profiles,
                entries = o.optJSONArray("entries")?.toObjectList()
                    ?.map(LeaderboardEntry::fromJson) ?: emptyList(),
            )
        }
    }
}

data class ThermalInfo(
    val celsius: Double?,
    val state: String,
) {
    companion object {
        fun fromJson(o: JSONObject): ThermalInfo = ThermalInfo(
            celsius = if (o.isNull("celsius")) null else o.optDouble("celsius").takeIf { !it.isNaN() },
            state = o.str("state", "unknown"),
        )
    }
}

data class ServerStatus(
    val ok: Boolean,
    val loadedModel: String?,
    val backend: String?,
    val engine: String?,
    val idleTimeoutSecs: Int,
    val thermal: ThermalInfo?,
    val lan: Boolean,
    val port: Int,
) {
    companion object {
        fun fromJson(o: JSONObject): ServerStatus = ServerStatus(
            ok = o.bool("ok", true),
            loadedModel = o.strOrNull("loaded_model"),
            backend = o.strOrNull("backend"),
            engine = o.strOrNull("engine"),
            idleTimeoutSecs = o.int("idle_timeout_secs"),
            thermal = o.optJSONObject("thermal")?.let(ThermalInfo::fromJson),
            lan = o.bool("lan"),
            port = o.int("port"),
        )
    }
}

data class VibeMarkResult(
    val model: String,
    val backend: String,
    val engine: String,
    val prefillTps: Double,
    val decodeTps: Double,
    val ttftMs: Double,
    val peakRamMb: Long,
    val durationS: Double,
    val provenance: String,
) {
    companion object {
        fun fromJson(o: JSONObject): VibeMarkResult = VibeMarkResult(
            model = o.str("model"),
            backend = o.str("backend"),
            engine = o.str("engine"),
            prefillTps = o.double("prefill_tps"),
            decodeTps = o.double("decode_tps"),
            ttftMs = o.double("ttft_ms"),
            peakRamMb = o.long("peak_ram_mb"),
            durationS = o.double("duration_s"),
            provenance = o.str("provenance", "measured"),
        )
    }
}

/** A .gguf file discovered in the models directory (internal storage or SAF). */
data class ModelFile(
    val name: String,
    /** Filesystem path when known; content URI string for SAF-only files. */
    val path: String,
    val sizeBytes: Long,
    val saf: Boolean,
)

/** One chat turn kept in the UI transcript. */
data class ChatMessage(
    val role: String,
    val content: String,
    val isError: Boolean = false,
)

object ChatRoles {
    const val USER = "user"
    const val ASSISTANT = "assistant"
    const val SYSTEM = "system"
}

/**
 * Extracts assistant text from a chat payload. Handles the OpenAI shape
 * (choices[0].message.content) as well as simpler {"content": "..."} /
 * {"response": "..."} / plain-string envelopes the bridge may return.
 */
fun extractChatContent(data: Any?): String? = when (data) {
    is String -> data.takeIf { it.isNotBlank() }
    is JSONObject -> {
        val choices = data.optJSONArray("choices")
        val first = choices?.optJSONObject(0)
        val message = first?.optJSONObject("message")
        message?.strOrNull("content")
            ?: first?.strOrNull("text")
            ?: data.firstString("content", "response", "text", "output")
    }
    else -> null
}

/** Builds the OpenAI-style messages array for POST /v1/chat/completions. */
fun buildMessagesArray(history: List<ChatMessage>): JSONArray {
    val arr = JSONArray()
    for (m in history) {
        if (m.isError) continue
        arr.put(JSONObject().put("role", m.role).put("content", m.content))
    }
    return arr
}