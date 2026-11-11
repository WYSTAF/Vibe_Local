package com.vibelocal.app.data

import android.content.Context

/** Persistent app settings backed by SharedPreferences. */
class AppPrefs(context: Context) {

    private val sp = context.getSharedPreferences("vibe_local_prefs", Context.MODE_PRIVATE)

    /** Remote Vibe-Link base URL used when the native library is unavailable. */
    var remoteHost: String
        get() = sp.getString(KEY_REMOTE_HOST, DEFAULT_REMOTE_HOST) ?: DEFAULT_REMOTE_HOST
        set(value) = sp.edit().putString(KEY_REMOTE_HOST, value).apply()

    /** Optional X-Vibe-Local-Token for the remote host. */
    var remoteToken: String
        get() = sp.getString(KEY_REMOTE_TOKEN, "") ?: ""
        set(value) = sp.edit().putString(KEY_REMOTE_TOKEN, value).apply()

    /** Token of the LAN server this device currently hosts (null when offline). */
    var lanToken: String?
        get() = sp.getString(KEY_LAN_TOKEN, null)
        set(value) = sp.edit().putString(KEY_LAN_TOKEN, value).apply()

    var idleTimeoutSecs: Int
        get() = sp.getInt(KEY_IDLE_TIMEOUT, DEFAULT_IDLE_TIMEOUT).coerceIn(MIN_IDLE_TIMEOUT, MAX_IDLE_TIMEOUT)
        set(value) = sp.edit().putInt(KEY_IDLE_TIMEOUT, value.coerceIn(MIN_IDLE_TIMEOUT, MAX_IDLE_TIMEOUT)).apply()

    var safTreeUri: String?
        get() = sp.getString(KEY_SAF_TREE, null)
        set(value) = sp.edit().putString(KEY_SAF_TREE, value).apply()

    var lastProfile: String
        get() = sp.getString(KEY_LAST_PROFILE, DEFAULT_PROFILE) ?: DEFAULT_PROFILE
        set(value) = sp.edit().putString(KEY_LAST_PROFILE, value).apply()

    var stripThink: Boolean
        get() = sp.getBoolean(KEY_STRIP_THINK, true)
        set(value) = sp.edit().putBoolean(KEY_STRIP_THINK, value).apply()

    companion object {
        const val DEFAULT_REMOTE_HOST = "http://127.0.0.1:20129"
        const val DEFAULT_PROFILE = "nothing_phone_2a"
        const val DEFAULT_IDLE_TIMEOUT = 300
        const val MIN_IDLE_TIMEOUT = 60
        const val MAX_IDLE_TIMEOUT = 1800

        private const val KEY_REMOTE_HOST = "remote_host"
        private const val KEY_REMOTE_TOKEN = "remote_token"
        private const val KEY_LAN_TOKEN = "lan_token"
        private const val KEY_IDLE_TIMEOUT = "idle_timeout_secs"
        private const val KEY_SAF_TREE = "saf_tree_uri"
        private const val KEY_LAST_PROFILE = "last_profile"
        private const val KEY_STRIP_THINK = "strip_think"
    }
}