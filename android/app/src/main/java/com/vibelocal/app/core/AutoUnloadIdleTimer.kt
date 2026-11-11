package com.vibelocal.app.core

import android.os.SystemClock
import com.vibelocal.app.data.AppPrefs
import com.vibelocal.app.data.VibeRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Guard #4 — idle auto-unload.
 *
 * Coroutine countdown, default 300 s, configurable 60–1800 s from the UI.
 * Every chat/load interaction calls [touch]. When the idle budget elapses the
 * timer syncs setIdleTimeout() into the native engine and unloads the model so
 * RAM is returned to the system. [arm] after a successful load, [disarm] after
 * an explicit unload.
 */
class AutoUnloadIdleTimer(
    private val repository: VibeRepository,
    private val scope: CoroutineScope,
    private val prefs: AppPrefs,
) {

    private val _timeoutSecs = MutableStateFlow(prefs.idleTimeoutSecs)
    val timeoutSecs: StateFlow<Int> = _timeoutSecs.asStateFlow()

    private val _armed = MutableStateFlow(false)
    val armed: StateFlow<Boolean> = _armed.asStateFlow()

    private var job: Job? = null

    @Volatile
    private var lastActivity: Long = SystemClock.elapsedRealtime()

    fun setTimeout(seconds: Int) {
        val clamped = seconds.coerceIn(AppPrefs.MIN_IDLE_TIMEOUT, AppPrefs.MAX_IDLE_TIMEOUT)
        prefs.idleTimeoutSecs = clamped
        _timeoutSecs.value = clamped
        scope.launch { repository.setIdleTimeout(clamped) }
        if (_armed.value) touch()
    }

    /** Start counting down (call after a model load succeeded). */
    fun arm() {
        _armed.value = true
        lastActivity = SystemClock.elapsedRealtime()
        scope.launch { repository.setIdleTimeout(_timeoutSecs.value) }
        restart()
    }

    /** Stop counting (call after an explicit unload). */
    fun disarm() {
        _armed.value = false
        job?.cancel()
        job = null
    }

    /** Reset the countdown; call on every chat/load interaction. */
    fun touch() {
        lastActivity = SystemClock.elapsedRealtime()
        if (_armed.value) restart()
    }

    private fun restart() {
        job?.cancel()
        job = scope.launch {
            while (_armed.value && isActive) {
                val timeoutMs = _timeoutSecs.value * 1000L
                val elapsed = SystemClock.elapsedRealtime() - lastActivity
                val remaining = timeoutMs - elapsed
                if (remaining <= 0L) {
                    repository.setIdleTimeout(_timeoutSecs.value)
                    repository.unloadModel()
                    _armed.value = false
                    break
                }
                delay(minOf(remaining, TICK_MS))
            }
        }
    }

    companion object {
        private const val TICK_MS = 1_000L
    }
}