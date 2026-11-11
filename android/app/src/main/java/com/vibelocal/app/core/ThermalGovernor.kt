package com.vibelocal.app.core

import android.content.Context
import android.os.Build
import android.os.PowerManager
import com.vibelocal.app.data.VibeRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.io.File

/**
 * Guard #2 — thermal protection.
 *
 * Three inputs:
 *  1. PowerManager.addThermalStatusListener (API 29+) for the OS thermal status.
 *  2. Device temperature read from /sys/class/thermal (SoC/CPU zones only),
 *     forwarded into the native governor via setThermalReading().
 *  3. The native governor's own view, read back through serverStatus()
 *     (native thresholds: 42°C throttle, 45°C pause).
 *
 * At THERMAL_STATUS_SEVERE or above (or >= 45°C) inference is paused: the
 * model is unloaded and [ThermalState.inferencePaused] tells the chat UI to
 * block sends and show a warning banner.
 */
class ThermalGovernor(
    private val context: Context,
    private val repository: VibeRepository,
    private val scope: CoroutineScope,
) {

    data class ThermalState(
        val powerStatus: Int,
        val deviceCelsius: Double?,
        val nativeCelsius: Double?,
        val nativeState: String?,
        val inferencePaused: Boolean,
        val throttling: Boolean,
    ) {
        /** Hottest trustworthy reading, native preferred when both exist. */
        val effectiveCelsius: Double?
            get() = listOfNotNull(nativeCelsius, deviceCelsius).maxOrNull()
    }

    private val powerManager =
        context.getSystemService(Context.POWER_SERVICE) as PowerManager

    private val _state = MutableStateFlow(
        ThermalState(
            powerStatus = PowerManager.THERMAL_STATUS_NONE,
            deviceCelsius = null,
            nativeCelsius = null,
            nativeState = null,
            inferencePaused = false,
            throttling = false,
        ),
    )
    val state: StateFlow<ThermalState> = _state.asStateFlow()

    private var job: Job? = null
    private var listener: PowerManager.OnThermalStatusChangedListener? = null

    fun start() {
        if (job != null) return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val l = PowerManager.OnThermalStatusChangedListener { status ->
                onPowerThermalStatus(status)
            }
            listener = l
            powerManager.addThermalStatusListener(l)
            onPowerThermalStatus(powerManager.currentThermalStatus)
        }
        job = scope.launch {
            while (isActive) {
                poll()
                delay(POLL_INTERVAL_MS)
            }
        }
    }

    fun stop() {
        job?.cancel()
        job = null
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            listener?.let { powerManager.removeThermalStatusListener(it) }
            listener = null
        }
    }

    private suspend fun poll() {
        val deviceC = readSysfsCelsius()
        if (deviceC != null) {
            // Feed the device reading into the native governor (42/45°C policy).
            repository.setThermalReading(deviceC)
        }
        val status = repository.status().getOrNull()
        recompute(
            deviceC = deviceC,
            nativeC = status?.thermal?.celsius,
            nativeState = status?.thermal?.state,
            powerStatus = _state.value.powerStatus,
        )
    }

    @Synchronized
    private fun onPowerThermalStatus(status: Int) {
        val current = _state.value
        recompute(current.deviceCelsius, current.nativeCelsius, current.nativeState, status)
    }

    @Synchronized
    private fun recompute(
        deviceC: Double?,
        nativeC: Double?,
        nativeState: String?,
        powerStatus: Int,
    ) {
        val effective = listOfNotNull(deviceC, nativeC).maxOrNull()
        val paused = powerStatus >= PowerManager.THERMAL_STATUS_SEVERE ||
            (effective != null && effective >= PAUSE_CELSIUS)
        val throttling = !paused && (
            powerStatus >= PowerManager.THERMAL_STATUS_MODERATE ||
                (effective != null && effective >= THROTTLE_CELSIUS)
            )
        val wasPaused = _state.value.inferencePaused
        _state.value = ThermalState(powerStatus, deviceC, nativeC, nativeState, paused, throttling)
        if (paused && !wasPaused) {
            // Rising edge into the danger zone: drop the model right away.
            scope.launch { repository.unloadModel() }
        }
    }

    /**
     * Reads SoC/CPU thermal zones. Returns null when nothing trustworthy is
     * readable (many devices restrict sysfs); never fabricates a value.
     */
    private fun readSysfsCelsius(): Double? {
        return try {
            val zones = File("/sys/class/thermal")
                .listFiles { f -> f.name.startsWith("thermal_zone") } ?: return null
            var hottest: Double? = null
            for (zone in zones) {
                val type = runCatching { File(zone, "type").readText().trim().lowercase() }
                    .getOrDefault("")
                val isSocZone = SOCC_KEYWORDS.any { type.contains(it) }
                if (!isSocZone) continue
                val raw = runCatching { File(zone, "temp").readText().trim().toLong() }
                    .getOrNull() ?: continue
                // Kernels report millidegrees; a few report whole degrees.
                val celsius = if (raw > 1000L) raw / 1000.0 else raw.toDouble()
                if (celsius <= 0.0 || celsius > 150.0) continue
                if (hottest == null || celsius > hottest) hottest = celsius
            }
            hottest
        } catch (e: Exception) {
            null
        }
    }

    companion object {
        /** Native governor throttle threshold. */
        const val THROTTLE_CELSIUS = 42.0

        /** Native governor pause threshold. */
        const val PAUSE_CELSIUS = 45.0

        private const val POLL_INTERVAL_MS = 5_000L
        private val SOCC_KEYWORDS = listOf(
            "cpu", "soc", "tsens", "exynos", "mtk", "apc", "cluster", "big", "little",
        )
    }
}