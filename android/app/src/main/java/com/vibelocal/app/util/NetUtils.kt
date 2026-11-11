package com.vibelocal.app.util

import java.net.Inet4Address
import java.net.NetworkInterface
import java.net.URL
import java.util.Collections

object NetUtils {

    /**
     * Best-effort LAN IPv4 address via NetworkInterface enumeration.
     * Prefers wlan*/eth* interfaces; falls back to any site-local IPv4.
     */
    fun lanIpv4(): String? {
        return try {
            val interfaces = NetworkInterface.getNetworkInterfaces() ?: return null
            var fallback: String? = null
            for (nif in Collections.list(interfaces)) {
                if (!nif.isUp || nif.isLoopback) continue
                for (addr in Collections.list(nif.inetAddresses)) {
                    if (addr !is Inet4Address) continue
                    if (addr.isLoopbackAddress || !addr.isSiteLocalAddress) continue
                    val host = addr.hostAddress ?: continue
                    val name = nif.name.lowercase()
                    if (name.startsWith("wlan") || name.startsWith("eth") || name.startsWith("ap")) {
                        return host
                    }
                    if (fallback == null) fallback = host
                }
            }
            fallback
        } catch (e: Exception) {
            null
        }
    }

    /**
     * Normalizes a user-entered host into a base URL.
     * Accepts "192.168.1.50:20129", "http://host:port", trailing slashes.
     * Returns null when the input cannot be a valid http(s) URL.
     */
    fun normalizeBaseUrl(raw: String): String? {
        val trimmed = raw.trim().trimEnd('/')
        if (trimmed.isEmpty()) return null
        val withScheme =
            if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) trimmed
            else "http://$trimmed"
        return try {
            val url = URL(withScheme)
            if (url.host.isNullOrBlank()) null else withScheme
        } catch (e: Exception) {
            null
        }
    }
}