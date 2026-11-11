package com.vibelocal.app.ui.vibemark

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.selection.selectable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.vibelocal.app.R
import com.vibelocal.app.core.ModelFile
import com.vibelocal.app.core.VibeMarkResult
import com.vibelocal.app.ui.common.EmptyHint
import com.vibelocal.app.ui.common.ErrorBanner
import com.vibelocal.app.ui.common.InfoBanner
import com.vibelocal.app.ui.common.KeyValueRow
import com.vibelocal.app.ui.common.SectionCard
import com.vibelocal.app.ui.leaderboard.LeaderboardViewModel
import com.vibelocal.app.ui.theme.AccentCyan
import com.vibelocal.app.ui.theme.OkGreen
import com.vibelocal.app.util.formatBytes

@Composable
fun VibeMarkScreen(vm: VibeMarkViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            SectionCard(title = stringResource(R.string.vm_pick_model)) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End,
                ) {
                    androidx.compose.material3.TextButton(onClick = vm::refreshModels) {
                        Icon(Icons.Outlined.Refresh, contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(stringResource(R.string.common_refresh))
                    }
                }
                if (state.models.isEmpty()) {
                    EmptyHint(stringResource(R.string.vm_no_models))
                } else {
                    state.models.forEach { model ->
                        ModelRow(
                            model = model,
                            selected = model.path == state.selectedPath,
                            enabled = !state.running,
                            onSelect = { vm.select(model.path) },
                        )
                    }
                }
            }
        }

        item {
            Button(
                onClick = vm::run,
                enabled = !state.running && state.selectedPath != null,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(Icons.Outlined.PlayArrow, contentDescription = null)
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.vm_run))
            }
        }

        if (state.running) {
            item {
                SectionCard {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        CircularProgressIndicator(modifier = Modifier.size(28.dp), strokeWidth = 3.dp)
                        Column {
                            Text(
                                text = stringResource(R.string.vm_running),
                                style = MaterialTheme.typography.titleMedium,
                            )
                            Text(
                                text = stringResource(R.string.vm_elapsed, state.elapsedSecs),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        }

        state.error?.let { error ->
            item { ErrorBanner(error) }
        }

        state.result?.let { result ->
            item { ResultCard(result) }

            item {
                SectionCard(title = stringResource(R.string.vm_profile_label)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        LeaderboardViewModel.PROFILES.forEach { profile ->
                            FilterChip(
                                selected = state.profile == profile,
                                onClick = { vm.setProfile(profile) },
                                label = { Text(profile, style = MaterialTheme.typography.labelSmall) },
                                colors = FilterChipDefaults.filterChipColors(
                                    selectedContainerColor = MaterialTheme.colorScheme.primaryContainer,
                                    selectedLabelColor = MaterialTheme.colorScheme.onPrimaryContainer,
                                ),
                            )
                        }
                    }
                    Button(
                        onClick = vm::submit,
                        enabled = !state.submitting && !state.submitted,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        if (state.submitting) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(16.dp),
                                strokeWidth = 2.dp,
                                color = MaterialTheme.colorScheme.onPrimary,
                            )
                            Spacer(Modifier.width(8.dp))
                            Text(stringResource(R.string.vm_submitting))
                        } else {
                            Text(stringResource(R.string.vm_submit))
                        }
                    }
                    if (state.submitted) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            Icon(
                                Icons.Outlined.CheckCircle,
                                contentDescription = null,
                                tint = OkGreen,
                                modifier = Modifier.size(18.dp),
                            )
                            Text(stringResource(R.string.vm_submitted), color = OkGreen)
                        }
                    }
                }
            }
        }

        if (state.result == null && !state.running && state.selectedPath == null) {
            item { InfoBanner(stringResource(R.string.vm_select_model_first)) }
        }
    }
}

@Composable
private fun ModelRow(model: ModelFile, selected: Boolean, enabled: Boolean, onSelect: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .selectable(
                selected = selected,
                enabled = enabled,
                role = Role.RadioButton,
                onClick = onSelect,
            )
            .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        RadioButton(selected = selected, onClick = null, enabled = enabled)
        Column(modifier = Modifier.weight(1f)) {
            Text(model.name, style = MaterialTheme.typography.bodyMedium)
            Text(
                text = formatBytes(model.sizeBytes),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun ResultCard(result: VibeMarkResult) {
    SectionCard(title = stringResource(R.string.vm_results_title)) {
        Text(
            text = result.model,
            style = MaterialTheme.typography.titleMedium,
            color = AccentCyan,
        )
        KeyValueRow(stringResource(R.string.vm_prefill), stringResource(R.string.units_tok_s, result.prefillTps))
        KeyValueRow(stringResource(R.string.vm_decode), stringResource(R.string.units_tok_s, result.decodeTps))
        KeyValueRow(stringResource(R.string.vm_ttft), stringResource(R.string.units_ms, result.ttftMs))
        KeyValueRow(stringResource(R.string.vm_peak_ram), stringResource(R.string.units_mb, result.peakRamMb))
        KeyValueRow(stringResource(R.string.vm_duration), stringResource(R.string.units_seconds, result.durationS))
        KeyValueRow(stringResource(R.string.vm_backend), result.backend)
        KeyValueRow(stringResource(R.string.vm_engine), result.engine)
        KeyValueRow(stringResource(R.string.vm_provenance), result.provenance)
    }
}