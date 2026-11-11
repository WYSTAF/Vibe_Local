package com.vibelocal.app.core

import android.app.ActivityManager
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.res.Configuration
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Guard #1 — lmkd-aware memory protection.
 *
 * Polls ActivityManager.MemoryInfo every 2 seconds and exposes a [StateFlow]
 * the UI observes. Model loads must pass [canLoadModel]: free RAM has to cover
 * the model size plus a 512 MB safety margin, otherwise Android's low-memory
 * killer would likely terminate us mid-inference.
 *
 * Also registers ComponentCallbacks2: at TRIM_MEMORY_RUNNING_CRITICAL (or a
 * global onLowMemory) the loaded model is unloaded immediately via
 * [onCriticalTrim].
 */
class LmkdMemoryGuard(
    private val context: Context,
    private val scope: CoroutineScope,
    private val onCriticalTrim: () -> Unit,
) : ComponentCallbacks2 {

    data class MemoryState(
        val availMem: Long,
        val totalMem: Long,
        val threshold: Long,
        val lowMemory: Boolean,
    ) {
        val availMb: Long get() = availMem / (1024L * 1024L)

        fun canLoadModel(sizeBytes: Long): Boolean =
            availMem >= sizeBytes + MARGIN_BYTES
    }

    private val activityManager =
        context.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager

    private val _state = MutableStateFlow(readMemoryInfo())
    val state: StateFlow<MemoryState> = _state.asStateFlow()

    private var job: Job? = null

    fun start() {
        if (job != null) return
        context.registerComponentCallbacks(this)
        job = scope.launch {
            while (isActive) {
                _state.value = readMemoryInfo()
                delay(POLL_INTERVAL_MS)
            }
        }
    }

    fun stop() {
        job?.cancel()
        job = null
        runCatching { context.unregisterComponentCallbacks(this) }
    }

    /** Fresh read (not the cached flow value) for load-time decisions. */
    fun canLoadModel(sizeBytes: Long): Boolean = readMemoryInfo().canLoadModel(sizeBytes)

    private fun readMemoryInfo(): MemoryState {
        val info = ActivityManager.MemoryInfo()
        activityManager.getMemoryInfo(info)
        return MemoryState(
            availMem = info.availMem,
            totalMem = info.totalMem,
            threshold = info.threshold,
            lowMemory = info.lowMemory,
        )
    }

    override fun onTrimMemory(level: Int) {
        if (level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL) {
            _state.value = readMemoryInfo()
            onCriticalTrim()
        }
    }

    override fun onLowMemory() {
        _state.value = readMemoryInfo()
        onCriticalTrim()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        // No-op: configuration changes do not affect memory policy.
    }

    companion object {
        /** Extra headroom required on top of the model file size. */
        const val MARGIN_BYTES = 512L * 1024L * 1024L
        private const val POLL_INTERVAL_MS = 2_000L
    }
}