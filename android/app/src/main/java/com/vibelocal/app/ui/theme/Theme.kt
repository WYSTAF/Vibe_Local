package com.vibelocal.app.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val ObsidianDarkScheme = darkColorScheme(
    primary = AccentViolet,
    onPrimary = Color.White,
    primaryContainer = AccentVioletDim,
    onPrimaryContainer = Color(0xFFD9CCFF),
    secondary = AccentCyan,
    onSecondary = Color(0xFF06222A),
    secondaryContainer = AccentCyanDim,
    onSecondaryContainer = Color(0xFFB8F1FB),
    tertiary = OkGreen,
    background = ObsidianBackground,
    onBackground = TextPrimary,
    surface = ObsidianSurface,
    onSurface = TextPrimary,
    surfaceVariant = ObsidianSurfaceVariant,
    onSurfaceVariant = TextSecondary,
    outline = ObsidianOutline,
    outlineVariant = Color(0xFF232941),
    error = DangerRed,
    onError = Color.White,
    errorContainer = DangerRedDim,
    onErrorContainer = Color(0xFFFFC2CE),
)

/**
 * Vibe Local is dark-only by design (Obsidian Dark). Edge-to-edge is enabled
 * in MainActivity; Scaffold handles window insets.
 */
@Composable
fun VibeLocalTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = ObsidianDarkScheme,
        typography = VibeTypography,
        content = content,
    )
}