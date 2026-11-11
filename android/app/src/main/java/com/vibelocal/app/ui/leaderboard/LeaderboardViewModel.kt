package com.vibelocal.app.ui.leaderboard

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import com.vibelocal.app.AppContainer
import com.vibelocal.app.VibeLocalApp
import com.vibelocal.app.core.LeaderboardData
import com.vibelocal.app.core.LeaderboardEntry
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class LbSortKey { QUALITY, POWER, PARAMS, NAME }

data class LeaderboardUiState(
    val profile: String = "",
    val loading: Boolean = false,
    val data: LeaderboardData? = null,
    val error: String? = null,
    val sortKey: LbSortKey = LbSortKey.QUALITY,
    val descending: Boolean = true,
    val expandedIds: Set<String> = emptySet(),
)

class LeaderboardViewModel(private val container: AppContainer) : ViewModel() {

    private val _state = MutableStateFlow(LeaderboardUiState(profile = container.prefs.lastProfile))
    val state: StateFlow<LeaderboardUiState> = _state.asStateFlow()

    init {
        load(_state.value.profile)
    }

    fun setProfile(profile: String) {
        if (profile == _state.value.profile && _state.value.data != null) return
        container.prefs.lastProfile = profile
        _state.update { it.copy(profile = profile, expandedIds = emptySet()) }
        load(profile)
    }

    fun refresh() {
        load(_state.value.profile)
    }

    private fun load(profile: String) {
        _state.update { it.copy(loading = true, error = null) }
        viewModelScope.launch {
            val result = container.repository.leaderboard(profile)
            _state.update {
                it.copy(
                    loading = false,
                    data = result.getOrNull() ?: it.data,
                    error = result.exceptionOrNull()?.message,
                )
            }
        }
    }

    fun setSortKey(key: LbSortKey) {
        _state.update {
            if (it.sortKey == key) {
                it.copy(descending = !it.descending)
            } else {
                it.copy(sortKey = key, descending = true)
            }
        }
    }

    fun toggleExpanded(id: String) {
        _state.update {
            val expanded = it.expandedIds.toMutableSet()
            if (!expanded.add(id)) expanded.remove(id)
            it.copy(expandedIds = expanded)
        }
    }

    /** Pure sorting helper used by the screen during composition. */
    fun sortedEntries(): List<LeaderboardEntry> {
        val s = _state.value
        val entries = s.data?.entries ?: return emptyList()
        val comparator: Comparator<LeaderboardEntry> = when (s.sortKey) {
            LbSortKey.QUALITY -> compareBy { it.vibeQualityScore }
            LbSortKey.POWER -> compareBy { it.powerPerParameter }
            LbSortKey.PARAMS -> compareBy { it.activeParamsB }
            LbSortKey.NAME -> compareBy { it.modelName.lowercase() }
        }
        val ordered = entries.sortedWith(comparator)
        return if (s.descending) ordered.reversed() else ordered
    }

    companion object {
        val PROFILES = listOf(
            "rx580_vulkan",
            "nothing_phone_2a",
            "i5_4690_ddr3",
            "tesla_t4",
        )

        val Factory: ViewModelProvider.Factory = viewModelFactory {
            initializer {
                val app = this[ViewModelProvider.AndroidViewModelFactory.APPLICATION_KEY] as VibeLocalApp
                LeaderboardViewModel(app.container)
            }
        }
    }
}