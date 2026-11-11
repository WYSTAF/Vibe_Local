package com.vibelocal.app.data

import android.app.DownloadManager
import android.content.Context
import android.net.Uri
import com.vibelocal.app.R
import com.vibelocal.app.core.ModelCandidate
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.io.File
import java.util.concurrent.ConcurrentHashMap

/**
 * Downloads candidate models with the system DownloadManager.
 *
 * DownloadManager can only write to external/app-external locations, so files
 * land in getExternalFilesDir("vibe_downloads") first and are then moved into
 * "<dataDir>/models" (internal storage or SAF USB OTG root) by [DataDirManager].
 *
 * Security: only URLs starting with https://huggingface.co/ are accepted.
 */
class DownloadController(
    private val context: Context,
    private val dataDirs: DataDirManager,
    private val scope: CoroutineScope,
) {

    enum class Status { QUEUED, RUNNING, IMPORTING, DONE, FAILED }

    data class DownloadState(
        val filename: String,
        val status: Status,
        /** 0..1 when the total size is known, null while indeterminate. */
        val progress: Float?,
        val message: String? = null,
        val modelPath: String? = null,
    )

    private val manager =
        context.getSystemService(Context.DOWNLOAD_SERVICE) as DownloadManager

    private val _states = MutableStateFlow<Map<String, DownloadState>>(emptyMap())
    val states: StateFlow<Map<String, DownloadState>> = _states.asStateFlow()

    private val activeIds = ConcurrentHashMap<String, Long>()
    private val jobs = ConcurrentHashMap<String, Job>()

    /**
     * Enqueues a download. Returns null on success, or a user-facing error
     * string when the request was refused.
     */
    fun start(candidate: ModelCandidate): String? {
        if (!candidate.isDownloadUrlTrusted) {
            return context.getString(R.string.download_invalid_url)
        }
        val filename = candidate.filename
        if (filename.isBlank()) return context.getString(R.string.common_unknown_error)
        if (activeIds.containsKey(filename)) return null
        if (dataDirs.listModels().any { it.name == filename }) {
            return context.getString(R.string.download_already_exists)
        }
        return try {
            // Clear any stale partial file from a previous attempt.
            stagingDir().resolve(filename).delete()
            val request = DownloadManager.Request(Uri.parse(candidate.downloadUrl))
                .setTitle(filename)
                .setDescription(context.getString(R.string.app_name))
                .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE)
                .setAllowedOverMetered(true)
                .setAllowedOverRoaming(true)
                .setDestinationInExternalFilesDir(context, STAGING_DIR, filename)
            val id = manager.enqueue(request)
            activeIds[filename] = id
            put(filename, DownloadState(filename, Status.QUEUED, 0f))
            jobs[filename] = scope.launch { poll(filename, id) }
            null
        } catch (e: Exception) {
            e.message ?: context.getString(R.string.common_unknown_error)
        }
    }

    fun cancel(filename: String) {
        activeIds[filename]?.let { manager.remove(it) }
        jobs[filename]?.cancel()
        activeIds.remove(filename)
        jobs.remove(filename)
        put(filename, DownloadState(filename, Status.FAILED, null, "Canceled"))
    }

    fun clearFinished() {
        _states.update { current ->
            current.filterValues { it.status == Status.RUNNING || it.status == Status.QUEUED || it.status == Status.IMPORTING }
        }
    }

    private fun stagingDir(): File =
        (context.getExternalFilesDir(STAGING_DIR) ?: File(context.filesDir, STAGING_DIR))
            .apply { mkdirs() }

    private suspend fun poll(filename: String, id: Long) {
        var terminal = false
        try {
            while (!terminal) {
                delay(POLL_INTERVAL_MS)
                val cursor = manager.query(DownloadManager.Query().setFilterById(id))
                if (cursor == null) {
                    fail(filename, "Download entry disappeared")
                    terminal = true
                    continue
                }
                cursor.use { c ->
                    if (!c.moveToFirst()) {
                        fail(filename, "Download entry disappeared")
                        terminal = true
                        return@use
                    }
                    val status = c.getInt(c.getColumnIndexOrThrow(DownloadManager.COLUMN_STATUS))
                    val soFar = c.getLong(c.getColumnIndexOrThrow(DownloadManager.COLUMN_BYTES_DOWNLOADED_SO_FAR))
                    val total = c.getLong(c.getColumnIndexOrThrow(DownloadManager.COLUMN_TOTAL_SIZE_BYTES))
                    when (status) {
                        DownloadManager.STATUS_PENDING,
                        DownloadManager.STATUS_PAUSED,
                        DownloadManager.STATUS_RUNNING,
                        -> {
                            val progress = if (total > 0) (soFar.toFloat() / total).coerceIn(0f, 1f) else null
                            put(filename, DownloadState(filename, Status.RUNNING, progress))
                        }

                        DownloadManager.STATUS_SUCCESSFUL -> {
                            terminal = true
                            put(filename, DownloadState(filename, Status.IMPORTING, 1f))
                            val staged = stagingDir().resolve(filename)
                            if (!staged.exists() || staged.length() == 0L) {
                                fail(filename, "Downloaded file is missing or empty")
                            } else {
                                val imported = dataDirs.importModel(staged, filename)
                                imported.fold(
                                    onSuccess = { model ->
                                        put(
                                            filename,
                                            DownloadState(
                                                filename, Status.DONE, 1f,
                                                context.getString(R.string.download_complete),
                                                model.path,
                                            ),
                                        )
                                    },
                                    onFailure = { e ->
                                        fail(
                                            filename,
                                            context.getString(R.string.download_import_failed, e.message ?: "?"),
                                        )
                                    },
                                )
                            }
                        }

                        DownloadManager.STATUS_FAILED -> {
                            terminal = true
                            val reason = c.getInt(c.getColumnIndexOrThrow(DownloadManager.COLUMN_REASON))
                            fail(filename, context.getString(R.string.download_failed, reasonText(reason)))
                        }
                    }
                }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            fail(filename, e.message ?: "Download error")
        } finally {
            activeIds.remove(filename)
            jobs.remove(filename)
        }
    }

    private fun fail(filename: String, message: String) {
        put(filename, DownloadState(filename, Status.FAILED, null, message))
    }

    private fun put(filename: String, state: DownloadState) {
        _states.update { it + (filename to state) }
    }

    private fun reasonText(reason: Int): String = when (reason) {
        DownloadManager.ERROR_FILE_ERROR -> "storage error"
        DownloadManager.ERROR_UNHANDLED_HTTP_CODE -> "unhandled HTTP code"
        DownloadManager.ERROR_HTTP_DATA_ERROR -> "HTTP data error"
        DownloadManager.ERROR_TOO_MANY_REDIRECTS -> "too many redirects"
        DownloadManager.ERROR_INSUFFICIENT_SPACE -> "insufficient space"
        DownloadManager.ERROR_DEVICE_NOT_FOUND -> "storage not found (USB unplugged?)"
        DownloadManager.ERROR_CANNOT_RESUME -> "cannot resume"
        else -> "code $reason"
    }

    companion object {
        private const val STAGING_DIR = "vibe_downloads"
        private const val POLL_INTERVAL_MS = 500L
    }
}