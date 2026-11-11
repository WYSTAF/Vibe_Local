package com.vibelocal.app.ui.scan

import android.net.Uri
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import com.vibelocal.app.AppContainer
import com.vibelocal.app.VibeLocalApp
import com.vibelocal.app.core.HardwareInfo
import com.vibelocal.app.core.ModelCandidate
import com.vibelocal.app.core.Recommendations
import com.vibelocal.app.data.DownloadController
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class ScanUiState(
    val scanning: Boolean = false,
    val hardware: HardwareInfo? = null,
    val recommendations: Recommendations? = null,
    val error: String? = null,
    val dataDirLabel: String = "",
    val isSaf: Boolean = false,
)

class ScanViewModel(private val container: AppContainer) : ViewModel() {

    private val _state = MutableStateFlow(
        ScanUiState(
            dataDirLabel = container.dataDirs.rootLabel,
            isSaf = container.dataDirs.isSaf,
        ),
    )
    val state: StateFlow<ScanUiState> = _state.asStateFlow()

    val downloads: StateFlow<Map<String, DownloadController.DownloadState>> =
        container.downloads.states

    init {
        // Keep the data-dir label in sync when the user switches to USB OTG.
        viewModelScope.launch {
            container.dataDirs.root.collect {
                _state.update {
                    it.copy(
                        dataDirLabel = container.dataDirs.rootLabel,
                        isSaf = container.dataDirs.isSaf,
                    )
                }
            }
        }
    }

    fun scan() {
        if (_state.value.scanning) return
        _state.update { it.copy(scanning = true, error = null) }
        viewModelScope.launch {
            val hardware = async { container.repository.scanHardware() }
            val recommendations = async { container.repository.recommendations() }
            val hwResult = hardware.await()
            val recResult = recommendations.await()
            val errors = listOfNotNull(
                hwResult.exceptionOrNull()?.message,
                recResult.exceptionOrNull()?.message,
            )
            _state.update {
                it.copy(
                    scanning = false,
                    hardware = hwResult.getOrNull() ?: it.hardware,
                    recommendations = recResult.getOrNull() ?: it.recommendations,
                    error = errors.takeIf { e -> e.isNotEmpty() }?.joinToString("\n"),
                )
            }
        }
    }

    fun download(candidate: ModelCandidate) {
        val error = container.downloads.start(candidate)
        if (error != null) {
            _state.update { it.copy(error = error) }
        }
    }

    fun cancelDownload(filename: String) {
        container.downloads.cancel(filename)
    }

    /** Returns false when the SAF grant/path derivation failed. */
    fun useSafRoot(uri: Uri): Boolean = container.dataDirs.useSafTree(uri)

    fun useInternalRoot() {
        container.dataDirs.useInternal()
    }

    fun setError(message: String) {
        _state.update { it.copy(error = message) }
    }

    fun clearError() {
        _state.update { it.copy(error = null) }
    }

    companion object {
        val Factory: ViewModelProvider.Factory = viewModelFactory {
            initializer {
                val app = this[ViewModelProvider.AndroidViewModelFactory.APPLICATION_KEY] as VibeLocalApp
                ScanViewModel(app.container)
            }
        }
    }
}