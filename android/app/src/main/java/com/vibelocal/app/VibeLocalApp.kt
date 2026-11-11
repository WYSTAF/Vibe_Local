package com.vibelocal.app

import android.app.Application
import android.content.Context
import com.vibelocal.app.core.AutoUnloadIdleTimer
import com.vibelocal.app.core.LanWorkerController
import com.vibelocal.app.core.LmkdMemoryGuard
import com.vibelocal.app.core.ThermalGovernor
import com.vibelocal.app.data.AppPrefs
import com.vibelocal.app.data.DataDirManager
import com.vibelocal.app.data.DownloadController
import com.vibelocal.app.data.VibeHttpClient
import com.vibelocal.app.data.VibeRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Manual dependency container (no DI framework needed at this size).
 * Owns the repository, the download pipeline and the four safety guards.
 */
class AppContainer(val appContext: Context) {

    val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val prefs = AppPrefs(appContext)
    val dataDirs = DataDirManager(appContext, prefs)
    val http = VibeHttpClient()
    val repository = VibeRepository(appContext, prefs, dataDirs, http)

    /** Guard #1: lmkd-aware memory policy. Critical trim unloads the model. */
    val memoryGuard = LmkdMemoryGuard(appContext, appScope) {
        appScope.launch { repository.unloadModel() }
    }

    /** Guard #2: OS + native thermal policy. */
    val thermalGovernor = ThermalGovernor(appContext, repository, appScope)

    /** Guard #4: idle auto-unload countdown. */
    val idleTimer = AutoUnloadIdleTimer(repository, appScope, prefs)

    /** LAN worker lifecycle (uses Guard #3, the foreground service). */
    val lanWorker = LanWorkerController(appContext, repository, prefs, appScope)

    val downloads = DownloadController(appContext, dataDirs, appScope)

    init {
        memoryGuard.start()
        thermalGovernor.start()
    }
}

class VibeLocalApp : Application() {

    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        container = AppContainer(this)
    }
}