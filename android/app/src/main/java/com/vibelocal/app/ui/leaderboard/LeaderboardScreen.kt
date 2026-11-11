package com.vibelocal.app.ui.leaderboard

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.ArrowDownward
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.vibelocal.app.R
import com.vibelocal.app.core.LeaderboardData
import com.vibelocal.app.core.LeaderboardEntry
import com.vibelocal.app.ui.common.ChipLabel
import com.vibelocal.app.ui.common.EmptyHint
import com.vibelocal.app.ui.common.ErrorBanner
import com.vibelocal.app.ui.common.KeyValueRow
import com.vibelocal.app.ui.common.SectionCard
import com.vibelocal.app.ui.theme.AccentCyan
import com.vibelocal.app.ui.theme.AccentViolet

@Composable
fun LeaderboardScreen(vm: LeaderboardViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val entries = vm.sortedEntries()

    Column(modifier = Modifier.fillMaxSize()) {
        // Profile switcher.
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 12.dp, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            LeaderboardViewModel.PROFILES.forEach { profile ->
                val label = state.data?.profiles?.get(profile)
                    ?: stringResource(profileLabelRes(profile))
                FilterChip(
                    selected = state.profile == profile,
                    onClick = { vm.setProfile(profile) },
                    label = { Text(label) },
                    colors = FilterChipDefaults.filterChipColors(
                        selectedContainerColor = MaterialTheme.colorScheme.primaryContainer,
                        selectedLabelColor = MaterialTheme.colorScheme.onPrimaryContainer,
                    ),
                )
            }
        }

        // Sort controls.
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            SortChip(stringResource(R.string.lb_sort_quality), LbSortKey.QUALITY, state.sortKey) { vm.setSortKey(it) }
            SortChip(stringResource(R.string.lb_sort_power), LbSortKey.POWER, state.sortKey) { vm.setSortKey(it) }
            SortChip(stringResource(R.string.lb_sort_params), LbSortKey.PARAMS, state.sortKey) { vm.setSortKey(it) }
            Spacer(Modifier.weight(1f))
            IconButton(onClick = { vm.setSortKey(state.sortKey) }) {
                Icon(
                    imageVector = if (state.descending) Icons.Outlined.ArrowDownward else Icons.Outlined.ArrowUpward,
                    contentDescription = stringResource(R.string.lb_sort_direction),
                    tint = AccentCyan,
                )
            }
            IconButton(onClick = vm::refresh) {
                Icon(
                    Icons.Outlined.Refresh,
                    contentDescription = stringResource(R.string.common_refresh),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        if (state.loading) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(8.dp),
                horizontalArrangement = Arrangement.Center,
            ) {
                CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
            }
        }

        LazyColumn(
            modifier = Modifier.weight(1f),
            contentPadding = PaddingValues(12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            state.error?.let { error ->
                item { ErrorBanner(error) }
            }

            item {
                SectionCard(title = stringResource(R.string.lb_pareto_title)) {
                    if (entries.isEmpty()) {
                        EmptyHint(stringResource(R.string.lb_pareto_empty))
                    } else {
                        ParetoChart(entries)
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Text(
                                text = stringResource(R.string.lb_pareto_x),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text(
                                text = stringResource(R.string.lb_pareto_y),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }

            item {
                Text(
                    text = stringResource(R.string.lb_entries_count, entries.size),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            if (entries.isEmpty() && !state.loading && state.error == null) {
                item { EmptyHint(stringResource(R.string.lb_empty)) }
            }

            itemsIndexed(entries, key = { index, entry -> "$index-${entry.id}" }) { _, entry ->
                EntryCard(
                    entry = entry,
                    data = state.data,
                    expanded = entry.id in state.expandedIds,
                    onToggle = { vm.toggleExpanded(entry.id) },
                    onOpenHf = { openHuggingFace(context, entry) },
                )
            }
        }
    }
}

@Composable
private fun SortChip(label: String, key: LbSortKey, current: LbSortKey, onSelect: (LbSortKey) -> Unit) {
    FilterChip(
        selected = current == key,
        onClick = { onSelect(key) },
        label = { Text(label, style = MaterialTheme.typography.labelSmall) },
        colors = FilterChipDefaults.filterChipColors(
            selectedContainerColor = MaterialTheme.colorScheme.secondaryContainer,
            selectedLabelColor = MaterialTheme.colorScheme.onSecondaryContainer,
        ),
    )
}

@Composable
private fun EntryCard(
    entry: LeaderboardEntry,
    data: LeaderboardData?,
    expanded: Boolean,
    onToggle: () -> Unit,
    onOpenHf: () -> Unit,
) {
    SectionCard {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggle),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(entry.modelName, style = MaterialTheme.typography.titleMedium)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    entry.quant?.let { ChipLabel(it, color = AccentCyan) }
                    entry.family?.let { ChipLabel(it.uppercase()) }
                    entry.backend?.let { ChipLabel(it) }
                }
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    text = String.format("%.1f", entry.vibeQualityScore),
                    style = MaterialTheme.typography.headlineSmall,
                    color = AccentViolet,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    text = stringResource(R.string.lb_quality_score),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.width(8.dp))
            Icon(
                imageVector = if (expanded) Icons.Outlined.ExpandLess else Icons.Outlined.ExpandMore,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        if (entry.paretoOptimal) {
            ChipLabel(stringResource(R.string.lb_pareto_optimal), color = AccentCyan)
        }

        KeyValueRow(
            stringResource(R.string.lb_power_per_param),
            String.format("%.3f", entry.powerPerParameter),
        )
        KeyValueRow(
            stringResource(R.string.lb_active_params),
            stringResource(R.string.units_params_b, entry.activeParamsB),
        )
        if (entry.totalParamsB > 0.0) {
            KeyValueRow(
                stringResource(R.string.lb_total_params),
                stringResource(R.string.units_params_b, entry.totalParamsB),
            )
        }
        entry.device?.let { KeyValueRow(stringResource(R.string.lb_device), it) }

        if (expanded) {
            Text(
                text = stringResource(R.string.lb_pillar_scores),
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.secondary,
            )
            val pillars = data?.pillars ?: LeaderboardData.DEFAULT_PILLARS
            val maxScore = (entry.pillarScores.values.maxOrNull() ?: 1.0).coerceAtLeast(1.0)
            pillars.forEach { pillar ->
                val label = data?.pillarLabels?.get(pillar) ?: prettifyPillar(pillar)
                val score = entry.pillarScores[pillar]
                Column(modifier = Modifier.fillMaxWidth()) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            text = label,
                            modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            text = score?.let { String.format("%.1f", it) }
                                ?: stringResource(R.string.common_not_available),
                            style = MaterialTheme.typography.bodyMedium,
                            fontWeight = FontWeight.SemiBold,
                        )
                    }
                    if (score != null) {
                        LinearProgressIndicator(
                            progress = { (score / maxScore).toFloat().coerceIn(0f, 1f) },
                            modifier = Modifier.fillMaxWidth(),
                            color = AccentViolet,
                            trackColor = MaterialTheme.colorScheme.surfaceVariant,
                        )
                    }
                }
            }
            entry.provenance?.let {
                KeyValueRow(stringResource(R.string.vm_provenance), it)
            }
        }

        if (entry.downloadUrl != null || entry.hfRepo != null) {
            TextButton(onClick = onOpenHf) {
                Icon(
                    Icons.AutoMirrored.Outlined.OpenInNew,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                )
                Spacer(Modifier.width(6.dp))
                Text(stringResource(R.string.lb_open_hf))
            }
        }
    }
}

private fun prettifyPillar(id: String): String =
    id.split('_').joinToString(" ") { part -> part.replaceFirstChar { it.uppercase() } }

private fun profileLabelRes(id: String): Int = when (id) {
    "rx580_vulkan" -> R.string.profile_rx580_vulkan
    "nothing_phone_2a" -> R.string.profile_nothing_phone_2a
    "i5_4690_ddr3" -> R.string.profile_i5_4690_ddr3
    "tesla_t4" -> R.string.profile_tesla_t4
    else -> R.string.app_name
}

/** Custom Tabs first, plain ACTION_VIEW as fallback. */
private fun openHuggingFace(context: Context, entry: LeaderboardEntry) {
    val url = entry.downloadUrl
        ?: entry.hfRepo?.let { "https://huggingface.co/$it" }
        ?: return
    val uri = Uri.parse(url)
    try {
        CustomTabsIntent.Builder().build().launchUrl(context, uri)
    } catch (e: Exception) {
        try {
            context.startActivity(Intent(Intent.ACTION_VIEW, uri))
        } catch (ignored: Exception) {
            // No browser available; nothing else we can do.
        }
    }
}