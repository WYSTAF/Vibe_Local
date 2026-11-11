package com.vibelocal.app.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import com.vibelocal.app.MainActivity
import com.vibelocal.app.R
import com.vibelocal.app.VibeLocalApp
import com.vibelocal.app.data.VibeRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Guard #3 — keeps the Vibe-Link LAN server alive while worker mode is on.
 *
 * - Notification channel "vibe_link", ongoing notification with a Stop action.
 * - START_STICKY so the system recreates the worker after process death
 *   (token is re-read from persisted prefs on a null-intent restart).
 * - Partial WakeLock (+ optional high-perf WifiLock) so inference serving is
 *   not suspended when the screen turns off.
 * - Calls startServer(lan=true) on start and stopServer() on destroy.
 *
 * Only ever started while the user has LAN worker mode enabled.
 */
class VibeLinkForegroundService : Service() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    private val container get() = (application as VibeLocalApp).container

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }

        val token = intent?.getStringExtra(EXTRA_TOKEN)
            ?: container.prefs.lanToken
            ?: ""
        val port = intent?.getIntExtra(EXTRA_PORT, VibeRepository.DEFAULT_PORT)
            ?: VibeRepository.DEFAULT_PORT

        // Must enter the foreground promptly, before the async server start.
        startInForeground(port)
        acquireLocks()

        scope.launch {
            val result = container.repository.startServer(port, lan = true, token = token)
            container.lanWorker.onServerStartResult(
                success = result.isSuccess,
                error = result.exceptionOrNull()?.message,
            )
            if (result.isFailure) {
                // Never keep a foreground service alive for a dead server.
                withContext(Dispatchers.Main) { stopSelf() }
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        // stopServer is a quick JNI call; run it synchronously so the listener
        // socket is really closed before the process may be killed.
        runCatching { SafeBridge.stopServer() }
        container.lanWorker.onServiceDestroyed()
        releaseLocks()
        scope.cancel()
        super.onDestroy()
    }

    // ------------------------------------------------------------ foreground

    private fun startInForeground(port: Int) {
        createChannel()
        val notification = buildNotification(port)
        ServiceCompat.startForeground(
            this,
            NOTIFICATION_ID,
            notification,
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )
    }

    private fun createChannel() {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.service_channel_name),
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = getString(R.string.service_channel_desc)
            setShowBadge(false)
        }
        manager.createNotificationChannel(channel)
    }

    private fun buildNotification(port: Int): Notification {
        val contentIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stopPendingIntent = PendingIntent.getService(
            this,
            1,
            stopIntent(this),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.service_notification_title))
            .setContentText(getString(R.string.service_notification_text, port))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(contentIntent)
            .addAction(0, getString(R.string.service_stop_action), stopPendingIntent)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }

    // ----------------------------------------------------------------- locks

    private fun acquireLocks() {
        if (wakeLock == null) {
            val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
            wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKELOCK_TAG).apply {
                setReferenceCounted(false)
                // Safety timeout aligned with the Android 15 dataSync FGS limit.
                acquire(MAX_LOCK_DURATION_MS)
            }
        }
        if (wifiLock == null) {
            runCatching {
                val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
                @Suppress("DEPRECATION")
                wifiLock = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, WIFILOCK_TAG).apply {
                    setReferenceCounted(false)
                    acquire()
                }
            }
        }
    }

    private fun releaseLocks() {
        runCatching { wakeLock?.takeIf { it.isHeld }?.release() }
        wakeLock = null
        runCatching { wifiLock?.takeIf { it.isHeld }?.release() }
        wifiLock = null
    }

    companion object {
        const val CHANNEL_ID = "vibe_link"
        const val ACTION_STOP = "com.vibelocal.app.action.STOP_VIBE_LINK"
        const val EXTRA_TOKEN = "token"
        const val EXTRA_PORT = "port"

        private const val NOTIFICATION_ID = 20129
        private const val WAKELOCK_TAG = "vibelocal:lan_worker"
        private const val WIFILOCK_TAG = "vibelocal:lan_wifi"
        private const val MAX_LOCK_DURATION_MS = 6L * 60L * 60L * 1000L

        fun startIntent(context: Context, token: String, port: Int): Intent =
            Intent(context, VibeLinkForegroundService::class.java).apply {
                putExtra(EXTRA_TOKEN, token)
                putExtra(EXTRA_PORT, port)
            }

        fun stopIntent(context: Context): Intent =
            Intent(context, VibeLinkForegroundService::class.java).apply {
                action = ACTION_STOP
            }
    }
}