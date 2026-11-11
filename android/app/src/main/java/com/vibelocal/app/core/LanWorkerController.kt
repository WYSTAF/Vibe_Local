package com.vibelocal.app.core

import android.content.Context
import androidx.core.content.ContextCompat
import com.vibelocal.app.R
import com.vibelocal.app.data.AppPrefs
import com.vibelocal.app.data.VibeRepository
import com.vibelocal.app.util.NetUtils
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.security.SecureRandom

/**
 * Coordinates LAN worker mode between the Chat tab UI and
 * [VibeLinkForegroundService]. Generates the 32-hex pairing token with
 * SecureRandom, discovers the device LAN IP, and tracks server lifecycle.
 */
class LanWorkerController(
    private val context: Context,
    private val repository: VibeRepository,
    private val prefs: AppPrefs,
    private val scope: CoroutineScope,
) {

    data class LanWorkerState(
        val enabled: Boolean = false,
        val starting: Boolean = false,
        val token: String? = null,
        val ip: String? = null,
        val port: Int = VibeRepository.DEFAULT_PORT,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(LanWorkerState())
    val state: StateFlow<LanWorkerState> = _state.asStateFlow()

    /** Generates a fresh token and starts the foreground service. */
    fun enable() {
        if (!repository.nativeAvailable) {
            _state.update { it.copy(error = context.getString(R.string.lan_native_required)) }
            return
        }
        if (_state.value.enabled || _state.value.starting) return

        val token = generateToken()
        prefs.lanToken = token
        val ip = NetUtils.lanIpv4()
        _state.update {
            it.copy(
                starting = true,
                token = token,
                ip = ip,
                error = if (ip == null) context.getString(R.string.lan_no_ip) else null,
            )
        }
        val intent = VibeLinkForegroundService.startIntent(
            context, token, VibeRepository.DEFAULT_PORT,
        )
        ContextCompat.startForegroundService(context, intent)
    }

    /** Stops the service (which calls stopServer in onDestroy) and cleans up. */
    fun disable() {
        context.stopService(VibeLinkForegroundService.stopIntent(context))
        scope.launch { repository.stopServer() }
        prefs.lanToken = null
        _state.update {
            LanWorkerState(port = it.port)
        }
    }

    fun clearError() {
        _state.update { it.copy(error = null) }
    }

    /** Called by the service once startServer() resolved. */
    fun onServerStartResult(success: Boolean, error: String?) {
        _state.update {
            if (success) {
                it.copy(enabled = true, starting = false, error = null)
            } else {
                it.copy(
                    enabled = false,
                    starting = false,
                    error = context.getString(R.string.lan_start_failed, error ?: "?"),
                )
            }
        }
    }

    /** Called from the service's onDestroy. */
    fun onServiceDestroyed() {
        prefs.lanToken = null
        _state.update { it.copy(enabled = false, starting = false) }
    }

    /** 16 random bytes -> 32 lowercase hex characters. */
    private fun generateToken(): String {
        val bytes = ByteArray(16)
        SecureRandom().nextBytes(bytes)
        return bytes.joinToString("") { "%02x".format(it) }
    }
}