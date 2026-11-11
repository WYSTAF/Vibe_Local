package com.vibelocal.app.ui

import android.os.PowerManager
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Forum
import androidx.compose.material.icons.outlined.Leaderboard
import androidx.compose.material.icons.outlined.Memory
import androidx.compose.material.icons.outlined.Speed
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.vibelocal.app.R
import com.vibelocal.app.VibeLocalApp
import com.vibelocal.app.core.ThermalGovernor
import com.vibelocal.app.ui.chat.ChatScreen
import com.vibelocal.app.ui.chat.ChatViewModel
import com.vibelocal.app.ui.leaderboard.LeaderboardScreen
import com.vibelocal.app.ui.leaderboard.LeaderboardViewModel
import com.vibelocal.app.ui.scan.ScanMatchScreen
import com.vibelocal.app.ui.scan.ScanViewModel
import com.vibelocal.app.ui.theme.AccentViolet
import com.vibelocal.app.ui.theme.AccentVioletDim
import com.vibelocal.app.ui.theme.ObsidianSurface
import com.vibelocal.app.ui.theme.TextSecondary
import com.vibelocal.app.ui.common.ErrorBanner
import com.vibelocal.app.ui.common.WarningBanner
import com.vibelocal.app.ui.vibemark.VibeMarkScreen
import com.vibelocal.app.ui.vibemark.VibeMarkViewModel

enum class VibeTab(val labelRes: Int, val icon: ImageVector) {
    SCAN(R.string.tab_scan, Icons.Outlined.Memory),
    LEADERBOARD(R.string.tab_leaderboard, Icons.Outlined.Leaderboard),
    VIBEMARK(R.string.tab_vibemark, Icons.Outlined.Speed),
    CHAT(R.string.tab_chat, Icons.Outlined.Forum),
}

@Composable
fun MainScreen() {
    val context = LocalContext.current
    val container = (context.applicationContext as VibeLocalApp).container

    val scanVm: ScanViewModel = viewModel(factory = ScanViewModel.Factory)
    val leaderboardVm: LeaderboardViewModel = viewModel(factory = LeaderboardViewModel.Factory)
    val vibeMarkVm: VibeMarkViewModel = viewModel(factory = VibeMarkViewModel.Factory)
    val chatVm: ChatViewModel = viewModel(factory = ChatViewModel.Factory)

    var selectedTab by rememberSaveable { mutableStateOf(VibeTab.SCAN) }

    val memory by container.memoryGuard.state.collectAsStateWithLifecycle()
    val thermal by container.thermalGovernor.state.collectAsStateWithLifecycle()

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        bottomBar = {
            NavigationBar(containerColor = ObsidianSurface, tonalElevation = 0.dp) {
                VibeTab.entries.forEach { tab ->
                    NavigationBarItem(
                        selected = selectedTab == tab,
                        onClick = { selectedTab = tab },
                        icon = { Icon(tab.icon, contentDescription = null) },
                        label = { Text(stringResource(tab.labelRes)) },
                        colors = NavigationBarItemDefaults.colors(
                            selectedIconColor = AccentViolet,
                            selectedTextColor = AccentViolet,
                            indicatorColor = AccentVioletDim,
                            unselectedIconColor = TextSecondary,
                            unselectedTextColor = TextSecondary,
                        ),
                    )
                }
            }
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            // Global safety banners (native fallback, thermal, memory).
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 4.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (!container.repository.nativeAvailable) {
                    ErrorBanner(
                        stringResource(R.string.banner_native_missing_title) + "\n" +
                            stringResource(
                                R.string.banner_native_missing_detail,
                                container.repository.nativeLoadError ?: "?",
                            ),
                    )
                }
                if (thermal.inferencePaused) {
                    WarningBanner(
                        stringResource(R.string.banner_thermal_paused, thermalDescription(thermal)),
                    )
                } else if (thermal.throttling) {
                    WarningBanner(
                        stringResource(R.string.banner_thermal_throttle, thermalDescription(thermal)),
                    )
                }
                if (memory.lowMemory) {
                    WarningBanner(stringResource(R.string.banner_low_memory))
                }
            }

            Box(modifier = Modifier.weight(1f)) {
                when (selectedTab) {
                    VibeTab.SCAN -> ScanMatchScreen(scanVm)
                    VibeTab.LEADERBOARD -> LeaderboardScreen(leaderboardVm)
                    VibeTab.VIBEMARK -> VibeMarkScreen(vibeMarkVm)
                    VibeTab.CHAT -> ChatScreen(chatVm)
                }
            }
        }
    }
}

@Composable
private fun thermalDescription(state: ThermalGovernor.ThermalState): String {
    val celsius = state.effectiveCelsius
    if (celsius != null) return stringResource(R.string.units_celsius, celsius)
    state.nativeState?.let { return it }
    return stringResource(thermalStatusRes(state.powerStatus))
}

private fun thermalStatusRes(status: Int): Int = when (status) {
    PowerManager.THERMAL_STATUS_NONE -> R.string.thermal_status_none
    PowerManager.THERMAL_STATUS_LIGHT -> R.string.thermal_status_light
    PowerManager.THERMAL_STATUS_MODERATE -> R.string.thermal_status_moderate
    PowerManager.THERMAL_STATUS_SEVERE -> R.string.thermal_status_severe
    PowerManager.THERMAL_STATUS_CRITICAL -> R.string.thermal_status_critical
    PowerManager.THERMAL_STATUS_EMERGENCY -> R.string.thermal_status_emergency
    PowerManager.THERMAL_STATUS_SHUTDOWN -> R.string.thermal_status_shutdown
    else -> R.string.thermal_status_none
}