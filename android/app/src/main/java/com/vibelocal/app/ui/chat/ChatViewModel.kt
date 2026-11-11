package com.vibelocal.app.ui.chat

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import com.vibelocal.app.AppContainer
import com.vibelocal.app.R
import com.vibelocal.app.VibeLocalApp
import com.vibelocal.app.core.ChatMessage
import com.vibelocal.app.core.ChatRoles
import com.vibelocal.app.core.LanWorkerController
import com.vibelocal.app.core.ModelFile
import com.vibelocal.app.core.ServerStatus
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

data class ChatUiState(
    val messages: List<ChatMessage> = emptyList(),
    val sending: Boolean = false,
    val stripThink: Boolean = true,
    val status: ServerStatus? = null,
    val models: List<ModelFile> = emptyList(),
    val selectedPath: String? = null,
    val backendOverride: String = "",
    val ngl: Int = 0,
    val ctx: Int = 4096,
    val loadingModel: Boolean = false,
    val loadError: String? = null,
    val idleTimeoutSecs: Int = 300,
)

class ChatViewModel(private val container: AppContainer) : ViewModel() {

    private val _state = MutableStateFlow(
        ChatUiState(
            stripThink = container.prefs.stripThink,
            idleTimeoutSecs = container.prefs.idleTimeoutSecs,
        ),
    )
    val state: StateFlow<ChatUiState> = _state.asStateFlow()

    val lanState: StateFlow<LanWorkerController.LanWorkerState> = container.lanWorker.state
    val remoteHost: StateFlow<String> = container.repository.remoteHost

    val nativeAvailable: Boolean get() = container.repository.nativeAvailable
    val remoteToken: String get() = container.repository.remoteToken

    init {
        refreshModels()
        viewModelScope.launch {
            container.idleTimer.timeoutSecs.collect { secs ->
                _state.update { it.copy(idleTimeoutSecs = secs) }
            }
        }
        // Keep the engine status card fresh while the app is in use.
        viewModelScope.launch {
            while (isActive) {
                refreshStatus()
                delay(STATUS_POLL_MS)
            }
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

    private suspend fun refreshStatus() {
        val status = container.repository.status().getOrNull()
        _state.update { it.copy(status = status) }
    }

    // ------------------------------------------------------------------ chat

    fun send(text: String) {
        val trimmed = text.trim()
        if (trimmed.isEmpty() || _state.value.sending) return

        if (container.thermalGovernor.state.value.inferencePaused) {
            appendError(container.appContext.getString(R.string.chat_paused_thermal))
            return
        }

        container.idleTimer.touch()
        val history = _state.value.messages + ChatMessage(ChatRoles.USER, trimmed)
        _state.update { it.copy(messages = history, sending = true) }

        viewModelScope.launch {
            // The JNI chat() takes a single prompt: flatten the transcript.
            // HTTP mode sends the full messages array instead.
            val prompt = history.filter { !it.isError }
                .joinToString("\n\n") { "${it.role}: ${it.content}" }
            val result = container.repository.chat(prompt, history, _state.value.stripThink)
            container.idleTimer.touch()
            result.fold(
                onSuccess = { content ->
                    _state.update {
                        it.copy(
                            messages = it.messages + ChatMessage(ChatRoles.ASSISTANT, content),
                            sending = false,
                        )
                    }
                },
                onFailure = { e ->
                    _state.update { it.copy(sending = false) }
                    appendError(e.message ?: container.appContext.getString(R.string.common_unknown_error))
                },
            )
            refreshStatus()
        }
    }

    private fun appendError(message: String) {
        _state.update {
            it.copy(messages = it.messages + ChatMessage(ChatRoles.ASSISTANT, message, isError = true))
        }
    }

    fun toggleStripThink() {
        val next = !_state.value.stripThink
        container.prefs.stripThink = next
        _state.update { it.copy(stripThink = next) }
    }

    // ----------------------------------------------------------------- model

    fun selectModel(path: String) {
        _state.update { it.copy(selectedPath = path, loadError = null) }
    }

    fun setBackendOverride(value: String) {
        _state.update { it.copy(backendOverride = value) }
    }

    fun setNgl(value: Int) {
        _state.update { it.copy(ngl = value.coerceIn(0, 999)) }
    }

    fun setCtx(value: Int) {
        _state.update { it.copy(ctx = value.coerceIn(512, 131072)) }
    }

    fun loadSelected() {
        val s = _state.value
        if (s.loadingModel) return
        val model = s.models.firstOrNull { it.path == s.selectedPath }
        if (model == null) {
            _state.update { it.copy(loadError = container.appContext.getString(R.string.chat_pick_model)) }
            return
        }
        // Guard #1: block loads that would push us into lmkd territory.
        if (!container.memoryGuard.canLoadModel(model.sizeBytes)) {
            val availMb = container.memoryGuard.state.value.availMb
            val needMb = model.sizeBytes / (1024L * 1024L)
            _state.update {
                it.copy(loadError = container.appContext.getString(R.string.chat_load_blocked_memory, availMb, needMb))
            }
            return
        }
        if (container.thermalGovernor.state.value.inferencePaused) {
            _state.update { it.copy(loadError = container.appContext.getString(R.string.chat_paused_thermal)) }
            return
        }
        _state.update { it.copy(loadingModel = true, loadError = null) }
        viewModelScope.launch {
            val backend = s.backendOverride.takeIf { it.isNotBlank() }
            val result = container.repository.loadModel(model.path, backend, s.ngl, s.ctx)
            result.fold(
                onSuccess = {
                    container.idleTimer.arm()
                    container.idleTimer.touch()
                    _state.update { it.copy(loadingModel = false) }
                    refreshStatus()
                },
                onFailure = { e ->
                    _state.update {
                        it.copy(
                            loadingModel = false,
                            loadError = container.appContext.getString(
                                R.string.chat_load_failed,
                                e.message ?: "?",
                            ),
                        )
                    }
                },
            )
        }
    }

    fun unload() {
        viewModelScope.launch {
            container.repository.unloadModel()
            container.idleTimer.disarm()
            refreshStatus()
        }
    }

    // -------------------------------------------------------------- settings

    fun setIdleTimeout(seconds: Int) {
        container.idleTimer.setTimeout(seconds)
    }

    fun setLanEnabled(enabled: Boolean) {
        if (enabled) container.lanWorker.enable() else container.lanWorker.disable()
    }

    /** Returns false when the host string is not a valid http(s) URL. */
    fun applyRemote(host: String, token: String): Boolean {
        val ok = container.repository.setRemoteHost(host)
        if (ok) container.repository.setRemoteToken(token)
        return ok
    }

    fun clearLoadError() {
        _state.update { it.copy(loadError = null) }
    }

    companion object {
        private const val STATUS_POLL_MS = 5_000L

        val Factory: ViewModelProvider.Factory = viewModelFactory {
            initializer {
                val app = this[ViewModelProvider.AndroidViewModelFactory.APPLICATION_KEY] as VibeLocalApp
                ChatViewModel(app.container)
            }
        }
    }
}