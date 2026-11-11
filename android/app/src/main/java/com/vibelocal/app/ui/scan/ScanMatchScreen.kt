package com.vibelocal.app.ui.scan

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.vibelocal.app.R
import com.vibelocal.app.core.HardwareInfo
import com.vibelocal.app.core.ModelCandidate
import com.vibelocal.app.core.Recommendations
import com.vibelocal.app.data.DownloadController
import com.vibelocal.app.ui.common.ChipLabel
import com.vibelocal.app.ui.common.ErrorBanner
import com.vibelocal.app.ui.common.KeyValueRow
import com.vibelocal.app.ui.common.SectionCard
import com.vibelocal.app.ui.common.WarningBanner
import com.vibelocal.app.ui.theme.AccentCyan
import com.vibelocal.app.ui.theme.OkGreen
import com.vibelocal.app.util.formatBytes

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ScanMatchScreen(vm: ScanViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val downloads by vm.downloads.collectAsStateWithLifecycle()
    val context = LocalContext.current

    val safLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocumentTree(),
    ) { uri ->
        if (uri != null && !vm.useSafRoot(uri)) {
            vm.setError(context.getString(R.string.data_dir_saf_failed))
        }
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Button(
                onClick = vm::scan,
                enabled = !state.scanning,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (state.scanning) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(18.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary,
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.scan_scanning))
                } else {
                    Icon(Icons.Outlined.Refresh, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.scan_button))
                }
            }
        }

        state.error?.let { error ->
            item { ErrorBanner(error) }
        }

        item {
            SectionCard(title = stringResource(R.string.data_dir_title)) {
                Text(
                    text = state.dataDirLabel,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = { safLauncher.launch(null) }) {
                        Icon(Icons.Outlined.FolderOpen, contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(stringResource(R.string.data_dir_choose_usb))
                    }
                    if (state.isSaf) {
                        TextButton(onClick = vm::useInternalRoot) {
                            Text(stringResource(R.string.data_dir_reset))
                        }
                    }
                }
            }
        }

        state.hardware?.let { hw ->
            item { HardwareCard(hw) }
        }

        state.recommendations?.let { rec ->
            item { VerdictCard(rec) }
            item {
                Text(
                    text = stringResource(R.string.scan_candidates_title),
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
            if (rec.candidates.isEmpty()) {
                item {
                    Text(
                        text = stringResource(R.string.scan_no_candidates),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            itemsIndexed(rec.candidates, key = { index, c -> "$index-${c.modelId}-${c.filename}" }) { _, candidate ->
                CandidateCard(
                    candidate = candidate,
                    download = downloads[candidate.filename],
                    onDownload = { vm.download(candidate) },
                    onCancel = { vm.cancelDownload(candidate.filename) },
                )
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun HardwareCard(hw: HardwareInfo) {
    SectionCard(title = stringResource(R.string.scan_hardware_section)) {
        KeyValueRow(stringResource(R.string.scan_os_arch), "${hw.os} / ${hw.arch}")
        KeyValueRow(stringResource(R.string.scan_cpu), hw.cpuModel)
        KeyValueRow(stringResource(R.string.scan_cpu_cores), hw.cpuCores.toString())
        KeyValueRow(stringResource(R.string.scan_ram_total), stringResource(R.string.units_mb, hw.ramTotalMb))
        KeyValueRow(stringResource(R.string.scan_ram_free), stringResource(R.string.units_mb, hw.ramFreeMb))
        KeyValueRow(stringResource(R.string.scan_preferred_backend), hw.preferredBackend)
        KeyValueRow(stringResource(R.string.scan_tier), hw.tier)
        hw.portable?.let { portable ->
            KeyValueRow(
                stringResource(R.string.scan_portable),
                if (portable.available) {
                    stringResource(R.string.scan_portable_available, portable.root ?: "?")
                } else {
                    stringResource(R.string.scan_portable_unavailable)
                },
            )
        }
        if (hw.cpuFlags.isNotEmpty()) {
            Text(
                text = stringResource(R.string.scan_cpu_flags),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                hw.cpuFlags.forEach { flag -> ChipLabel(flag) }
            }
        }
        Text(
            text = stringResource(R.string.scan_gpu_section),
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.secondary,
        )
        if (hw.gpus.isEmpty()) {
            Text(
                text = stringResource(R.string.scan_no_gpus),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } else {
            hw.gpus.forEach { gpu ->
                SectionCard {
                    Text(gpu.name, style = MaterialTheme.typography.titleMedium)
                    KeyValueRow(stringResource(R.string.gpu_vendor), gpu.vendor)
                    KeyValueRow(stringResource(R.string.gpu_vram), stringResource(R.string.units_mb, gpu.vramMb))
                    KeyValueRow(stringResource(R.string.gpu_backend), gpu.backend)
                }
            }
        }
    }
}

@Composable
private fun VerdictCard(rec: Recommendations) {
    SectionCard(title = stringResource(R.string.scan_verdict_title)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                text = if (rec.prefersMoe) {
                    stringResource(R.string.scan_verdict_moe)
                } else {
                    stringResource(R.string.scan_verdict_dense)
                },
                style = MaterialTheme.typography.titleMedium,
                color = if (rec.prefersMoe) AccentCyan else MaterialTheme.colorScheme.primary,
            )
            ChipLabel(rec.tier, color = AccentCyan)
        }
        Text(
            text = rec.rationale,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun CandidateCard(
    candidate: ModelCandidate,
    download: DownloadController.DownloadState?,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
) {
    SectionCard {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(
                text = candidate.modelId,
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.weight(1f),
            )
            ChipLabel(candidate.quant, color = AccentCyan)
            ChipLabel(candidate.family.uppercase())
        }
        Text(
            text = candidate.filename,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        KeyValueRow(stringResource(R.string.candidate_size), formatBytes(candidate.sizeBytes))
        KeyValueRow(
            stringResource(R.string.candidate_active_params),
            stringResource(R.string.units_params_b, candidate.activeParamsB),
        )
        KeyValueRow(
            stringResource(R.string.candidate_total_params),
            stringResource(R.string.units_params_b, candidate.totalParamsB),
        )
        KeyValueRow(stringResource(R.string.candidate_est_ram), stringResource(R.string.units_mb, candidate.estRamMb))
        KeyValueRow(stringResource(R.string.candidate_fit), candidate.fit)
        KeyValueRow(stringResource(R.string.candidate_ngl), candidate.ngl.toString())
        if (candidate.estSpeedupVsDense7b > 0.0) {
            Text(
                text = stringResource(R.string.units_x_speedup, candidate.estSpeedupVsDense7b),
                style = MaterialTheme.typography.bodyLarge,
                color = AccentCyan,
            )
        }
        if (candidate.reasons.isNotEmpty()) {
            Text(
                text = stringResource(R.string.candidate_reasons),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            candidate.reasons.forEach { reason ->
                Text(
                    text = "• $reason",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                )
            }
        }

        when (download?.status) {
            null, DownloadController.Status.FAILED -> {
                if (!candidate.isDownloadUrlTrusted) {
                    WarningBanner(stringResource(R.string.download_invalid_url))
                } else {
                    Button(onClick = onDownload, modifier = Modifier.fillMaxWidth()) {
                        Icon(Icons.Outlined.Download, contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(stringResource(R.string.common_download))
                    }
                }
                download?.message?.let { ErrorBanner(it) }
            }

            DownloadController.Status.QUEUED, DownloadController.Status.RUNNING -> {
                val progress = download?.progress
                if (progress != null) {
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Text(
                        text = "${(progress * 100).toInt()}%",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                } else {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    Text(
                        text = stringResource(R.string.common_downloading),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                TextButton(onClick = onCancel) { Text(stringResource(R.string.common_cancel)) }
            }

            DownloadController.Status.IMPORTING -> {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    CircularProgressIndicator(modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                    Text(stringResource(R.string.common_loading))
                }
            }

            DownloadController.Status.DONE -> {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Icon(
                        Icons.Outlined.CheckCircle,
                        contentDescription = null,
                        tint = OkGreen,
                        modifier = Modifier.size(18.dp),
                    )
                    Text(
                        text = stringResource(R.string.common_downloaded),
                        color = OkGreen,
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }
        }
    }
}