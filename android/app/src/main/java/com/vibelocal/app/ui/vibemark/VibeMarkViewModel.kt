package com.vibelocal.app.ui.vibemark

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import com.vibelocal.app.AppContainer
import com.vibelocal.app.R
import com.vibelocal.app.VibeLocalApp
import com.vibelocal.app.core.HardwareInfo
import com.vibelocal.app.core.ModelFile
import com.vibelocal.app.core.VibeMarkResult
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject

data class VibeMarkUiState(
    val models: List<ModelFile> = emptyList(),
    val selectedPath: String? = null,
    val running: Boolean = false,
    val elapsedSecs: Int = 0,
    val result: VibeMarkResult? = null,
    val error: String? = null,
    val submitting: Boolean = false,
    val submitted: Boolean = false,
    val profile: String = "",
)

class VibeMarkViewModel(private val container: AppContainer) : ViewModel() {

    private val _state = MutableStateFlow(VibeMarkUiState(profile = container.prefs.lastProfile))
    val state: StateFlow<VibeMarkUiState> = _state.asStateFlow()

    /** Cached hardware scan used to enrich submitted entries. */
    private var hardware: HardwareInfo? = null

    init {
        refreshModels()
        viewModelScope.launch {
            hardware = container.repository.scanHardware().getOrNull()
        }
    }

    fun refreshModels() {
        val models = container.repository.listModels()
        _state.update {
            val selected = it.selectedPath?.takeIf { p -> models.any { m -> m.path == p } }
                ?: models.firstOrNull()?.path
            it.copy(models = models, selectedPath = selected)
        }
    }

    fun select(path: String) {
        _state.update { it.copy(selectedPath = path) }
    }

    fun setProfile(profile: String) {
        container.prefs.lastProfile = profile
        _state.update { it.copy(profile = profile) }
    }

    fun run() {
        val s = _state.value
        val path = s.selectedPath
        if (path == null) {
            _state.update { it.copy(error = container.appContext.getString(R.string.vm_select_model_first)) }
            return
        }
        if (s.running) return
        _state.update {
            it.copy(running = true, elapsedSecs = 0, result = null, error = null, submitted = false)
        }
        viewModelScope.launch {
            val ticker: Job = launch {
                while (isActive) {
                    delay(1_000)
                    _state.update { st -> st.copy(elapsedSecs = st.elapsedSecs + 1) }
                }
            }
            container.idleTimer.touch()
            val result = container.repository.runVibeMark(path, BUDGET_SECS)
            ticker.cancel()
            result.fold(
                onSuccess = { r ->
                    _state.update { it.copy(running = false, result = r) }
                    // VibeMark leaves the model loaded; arm the idle guard.
                    container.idleTimer.arm()
                },
                onFailure = { e ->
                    _state.update {
                        it.copy(running = false, error = e.message ?: container.appContext.getString(R.string.common_unknown_error))
                    }
                },
            )
        }
    }

    fun submit() {
        val s = _state.value
        val result = s.result ?: return
        if (s.submitting || s.submitted) return
        _state.update { it.copy(submitting = true, error = null) }
        viewModelScope.launch {
            val entry = buildEntryJson(result, s)
            val outcome = container.repository.submitLeaderboard(entry.toString())
            _state.update {
                it.copy(
                    submitting = false,
                    submitted = outcome.isSuccess,
                    error = outcome.exceptionOrNull()?.message,
                )
            }
        }
    }

    /**
     * Submission payload: measured throughput plus provenance and hardware
     * context. The server derives pillar scores; the client never invents
     * quality numbers.
     */
    private fun buildEntryJson(result: VibeMarkResult, s: VibeMarkUiState): JSONObject {
        val filename = s.models.firstOrNull { it.path == s.selectedPath }?.name
            ?: result.model
        val entry = JSONObject()
            .put("model_id", filename.removeSuffix(".gguf"))
            .put("model", result.model.ifBlank { filename })
            .put("filename", filename)
            .put("profile", s.profile)
            .put("backend", result.backend)
            .put("engine", result.engine)
            .put("prefill_tps", result.prefillTps)
            .put("decode_tps", result.decodeTps)
            .put("ttft_ms", result.ttftMs)
            .put("peak_ram_mb", result.peakRamMb)
            .put("duration_s", result.durationS)
            .put("budget_secs", BUDGET_SECS)
            .put("provenance", result.provenance)
            .put("submitted_at_ms", System.currentTimeMillis())
        hardware?.let { hw ->
            entry.put(
                "hardware",
                JSONObject()
                    .put("os", hw.os)
                    .put("arch", hw.arch)
                    .put("cpu_model", hw.cpuModel)
                    .put("cpu_cores", hw.cpuCores)
                    .put("ram_total_mb", hw.ramTotalMb)
                    .put("preferred_backend", hw.preferredBackend)
                    .put("tier", hw.tier),
            )
        }
        return entry
    }

    fun clearError() {
        _state.update { it.copy(error = null) }
    }

    companion object {
        const val BUDGET_SECS = 15

        val Factory: ViewModelProvider.Factory = viewModelFactory {
            initializer {
                val app = this[ViewModelProvider.AndroidViewModelFactory.APPLICATION_KEY] as VibeLocalApp
                VibeMarkViewModel(app.container)
            }
        }
    }
}