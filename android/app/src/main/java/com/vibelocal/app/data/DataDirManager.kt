package com.vibelocal.app.data

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.DocumentsContract
import androidx.documentfile.provider.DocumentFile
import com.vibelocal.app.core.ModelFile
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File

/**
 * Resolves the Vibe Local data directory.
 *
 * Default: context.filesDir/vibe_data (internal, always writable).
 * Optional: a user-chosen USB OTG root via Storage Access Framework with a
 * persisted URI permission. Java-side I/O then goes through DocumentFile;
 * the native engine receives the derived /storage/<volume> filesystem path.
 *
 * Models always live in "<dataDir>/models".
 */
class DataDirManager(
    private val context: Context,
    private val prefs: AppPrefs,
) {

    sealed class Root {
        data class Internal(val dir: File) : Root()
        data class Saf(val treeUri: Uri, val fsPath: String) : Root()
    }

    private val _root = MutableStateFlow<Root>(restore())
    val root: StateFlow<Root> = _root.asStateFlow()

    /** Human readable location for the UI. */
    val rootLabel: String
        get() = when (val r = _root.value) {
            is Root.Internal -> r.dir.absolutePath
            is Root.Saf -> r.fsPath
        }

    /** Filesystem path handed to native startServer()/load/vibemark. */
    val dataDirPath: String
        get() = when (val r = _root.value) {
            is Root.Internal -> r.dir.absolutePath
            is Root.Saf -> r.fsPath
        }

    val isSaf: Boolean get() = _root.value is Root.Saf

    // ------------------------------------------------------------- switching

    fun useInternal() {
        prefs.safTreeUri = null
        _root.value = Root.Internal(internalDir())
    }

    /**
     * Persists access to a tree picked with ACTION_OPEN_DOCUMENT_TREE.
     * Returns false when the permission grant or path derivation fails.
     */
    fun useSafTree(uri: Uri): Boolean {
        return try {
            context.contentResolver.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
            )
            val path = deriveFsPath(uri) ?: return false
            val dir = File(path)
            if (!dir.exists() || !dir.isDirectory) return false
            prefs.safTreeUri = uri.toString()
            _root.value = Root.Saf(uri, path)
            true
        } catch (e: Exception) {
            false
        }
    }

    private fun restore(): Root {
        val saved = prefs.safTreeUri
        if (saved != null) {
            try {
                val uri = Uri.parse(saved)
                val path = deriveFsPath(uri)
                if (path != null && File(path).isDirectory) {
                    return Root.Saf(uri, path)
                }
            } catch (e: Exception) {
                // Fall through to internal storage.
            }
        }
        return Root.Internal(internalDir())
    }

    private fun internalDir(): File =
        context.filesDir.resolve("vibe_data").apply { mkdirs() }

    // ---------------------------------------------------------------- models

    /** The models directory as a java File, when the mount allows path access. */
    fun modelsDirFile(): File? = when (val r = _root.value) {
        is Root.Internal -> File(r.dir, "models").apply { mkdirs() }
        is Root.Saf -> File(r.fsPath, "models").apply { mkdirs() }
    }

    /** Lists .gguf files, preferring fast filesystem access, falling back to SAF. */
    fun listModels(): List<ModelFile> {
        val r = _root.value
        val viaFs = runCatching {
            modelsDirFile()
                ?.listFiles()
                ?.filter { it.isFile && it.name.endsWith(".gguf", ignoreCase = true) }
                ?.map { ModelFile(it.name, it.absolutePath, it.length(), saf = r is Root.Saf) }
                .orEmpty()
        }.getOrDefault(emptyList())
        if (viaFs.isNotEmpty()) return viaFs.sortedBy { it.name.lowercase() }

        if (r is Root.Saf) {
            return runCatching {
                val tree = DocumentFile.fromTreeUri(context, r.treeUri) ?: return emptyList()
                val models = tree.findFile("models") ?: return emptyList()
                models.listFiles()
                    .filter { it.isFile && (it.name?.endsWith(".gguf", ignoreCase = true) == true) }
                    .mapNotNull { doc ->
                        val name = doc.name ?: return@mapNotNull null
                        ModelFile(
                            name = name,
                            path = File(r.fsPath, "models/$name").absolutePath,
                            sizeBytes = doc.length(),
                            saf = true,
                        )
                    }
                    .sortedBy { it.name.lowercase() }
            }.getOrDefault(emptyList())
        }
        return emptyList()
    }

    /**
     * Moves a finished download into "<dataDir>/models". Uses the filesystem
     * when possible and falls back to a DocumentFile stream copy for SAF roots.
     */
    fun importModel(temp: File, filename: String): Result<ModelFile> = runCatching {
        when (val r = _root.value) {
            is Root.Internal -> {
                val dir = File(r.dir, "models").apply { mkdirs() }
                val target = File(dir, filename)
                if (target.exists()) target.delete()
                if (!temp.renameTo(target)) {
                    temp.copyTo(target, overwrite = true)
                    temp.delete()
                }
                ModelFile(target.name, target.absolutePath, target.length(), saf = false)
            }

            is Root.Saf -> {
                // Fast path: direct copy when the volume mount permits it.
                val direct = runCatching {
                    val dir = File(r.fsPath, "models").apply { mkdirs() }
                    val target = File(dir, filename)
                    temp.copyTo(target, overwrite = true)
                    ModelFile(target.name, target.absolutePath, target.length(), saf = true)
                }.getOrNull()
                if (direct != null && direct.sizeBytes > 0) {
                    temp.delete()
                    return@runCatching direct
                }
                // Slow path: SAF stream copy.
                val tree = DocumentFile.fromTreeUri(context, r.treeUri)
                    ?: error("SAF tree is no longer accessible")
                val modelsDir = tree.findFile("models")
                    ?: tree.createDirectory("models")
                    ?: error("Could not create models directory on the USB volume")
                modelsDir.findFile(filename)?.delete()
                val doc = modelsDir.createFile("application/octet-stream", filename)
                    ?: error("Could not create $filename on the USB volume")
                context.contentResolver.openOutputStream(doc.uri, "w")?.use { out ->
                    temp.inputStream().use { input -> input.copyTo(out) }
                } ?: error("Could not open output stream for $filename")
                temp.delete()
                ModelFile(
                    name = doc.name ?: filename,
                    path = File(r.fsPath, "models/$filename").absolutePath,
                    sizeBytes = doc.length(),
                    saf = true,
                )
            }
        }
    }

    fun deleteModel(name: String): Boolean {
        val r = _root.value
        val fsDeleted = runCatching {
            val f = File(modelsDirFile(), name)
            f.exists() && f.delete()
        }.getOrDefault(false)
        if (fsDeleted) return true
        if (r is Root.Saf) {
            return runCatching {
                DocumentFile.fromTreeUri(context, r.treeUri)
                    ?.findFile("models")
                    ?.findFile(name)
                    ?.delete() ?: false
            }.getOrDefault(false)
        }
        return false
    }

    companion object {
        /**
         * Derives a filesystem path from an external-storage tree URI.
         * "1234-ABCD:sub/dir" -> /storage/1234-ABCD/sub/dir
         * "primary:dir"       -> /storage/emulated/0/dir
         * Returns null for non-externalstorage providers or unmounted volumes.
         */
        fun deriveFsPath(treeUri: Uri): String? {
            return try {
                if (treeUri.authority != "com.android.externalstorage.documents") return null
                val docId = DocumentsContract.getTreeDocumentId(treeUri)
                val parts = docId.split(":", limit = 2)
                val volume = parts[0]
                val rel = parts.getOrElse(1) { "" }
                val base = if (volume.equals("primary", ignoreCase = true)) {
                    Environment.getExternalStorageDirectory().absolutePath
                } else {
                    "/storage/$volume"
                }
                val path = if (rel.isEmpty()) base else "$base/$rel"
                File(path).takeIf { it.exists() }?.absolutePath
            } catch (e: Exception) {
                null
            }
        }
    }
}