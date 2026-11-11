package com.vibelocal.app.ui.leaderboard

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.unit.dp
import com.vibelocal.app.core.LeaderboardEntry
import com.vibelocal.app.ui.theme.AccentCyan
import com.vibelocal.app.ui.theme.AccentViolet
import com.vibelocal.app.ui.theme.ObsidianOutline
import com.vibelocal.app.ui.theme.TextSecondary
import kotlin.math.max

/**
 * Compose Canvas Pareto scatter: x = active_params_b, y = vibe_quality_score.
 * Entries flagged pareto_optimal are highlighted and connected as the
 * frontier. When the server flags nothing, the skyline is computed locally
 * (max quality for increasing active params) so the chart stays useful.
 */
@Composable
fun ParetoChart(entries: List<LeaderboardEntry>, modifier: Modifier = Modifier) {
    val points = entries.filter { it.activeParamsB > 0.0 || it.vibeQualityScore > 0.0 }
    if (points.isEmpty()) return

    val flagged = points.filter { it.paretoOptimal }
    val optimalIds: Set<String> =
        if (flagged.isNotEmpty()) flagged.map { it.id }.toSet()
        else computeSkyline(points).map { it.id }.toSet()

    val accent = AccentViolet
    val cyan = AccentCyan
    val grid = ObsidianOutline
    val labelColor = TextSecondary

    Canvas(
        modifier = modifier
            .fillMaxWidth()
            .height(240.dp),
    ) {
        val padLeft = 40.dp.toPx()
        val padRight = 12.dp.toPx()
        val padTop = 12.dp.toPx()
        val padBottom = 26.dp.toPx()
        val plotW = size.width - padLeft - padRight
        val plotH = size.height - padTop - padBottom
        if (plotW <= 0 || plotH <= 0) return@Canvas

        val xMax = max(points.maxOf { it.activeParamsB } * 1.1, 1.0)
        val yMax = max(points.maxOf { it.vibeQualityScore } * 1.1, 1.0)

        fun px(x: Double): Float = (padLeft + (x / xMax) * plotW).toFloat()
        fun py(y: Double): Float = (padTop + plotH - (y / yMax) * plotH).toFloat()

        val paint = android.graphics.Paint().apply {
            color = labelColor.toArgb()
            textSize = 10.dp.toPx()
            isAntiAlias = true
        }

        // Horizontal grid + y labels.
        for (i in 0..4) {
            val value = yMax * i / 4.0
            val y = py(value)
            drawLine(grid, Offset(padLeft, y), Offset(size.width - padRight, y), strokeWidth = 1f)
            drawContext.canvas.nativeCanvas.drawText(
                String.format("%.0f", value),
                4.dp.toPx(),
                y + 4.dp.toPx(),
                paint,
            )
        }
        // X axis extent labels.
        drawContext.canvas.nativeCanvas.drawText("0", padLeft, size.height - 6.dp.toPx(), paint)
        drawContext.canvas.nativeCanvas.drawText(
            String.format("%.1f", xMax),
            size.width - padRight - 28.dp.toPx(),
            size.height - 6.dp.toPx(),
            paint,
        )

        // Frontier through the optimal points, sorted by active params.
        val frontier = points.filter { it.id in optimalIds }.sortedBy { it.activeParamsB }
        if (frontier.size >= 2) {
            val path = Path()
            frontier.forEachIndexed { index, e ->
                val offset = Offset(px(e.activeParamsB), py(e.vibeQualityScore))
                if (index == 0) path.moveTo(offset.x, offset.y) else path.lineTo(offset.x, offset.y)
            }
            drawPath(path, cyan, style = Stroke(width = 1.5.dp.toPx()))
        }

        // Scatter points.
        points.forEach { e ->
            val center = Offset(px(e.activeParamsB), py(e.vibeQualityScore))
            val optimal = e.id in optimalIds
            if (optimal) {
                drawCircle(cyan, radius = 9.dp.toPx(), center = center, style = Stroke(width = 1.5.dp.toPx()))
                drawCircle(accent, radius = 5.dp.toPx(), center = center)
            } else {
                drawCircle(grid, radius = 4.dp.toPx(), center = center)
            }
        }
    }
}

/** Local Pareto skyline: strictly better quality at increasing active params. */
private fun computeSkyline(points: List<LeaderboardEntry>): List<LeaderboardEntry> {
    val sorted = points.sortedWith(
        compareBy<LeaderboardEntry> { it.activeParamsB }.thenByDescending { it.vibeQualityScore },
    )
    val result = mutableListOf<LeaderboardEntry>()
    var bestQuality = Double.NEGATIVE_INFINITY
    for (e in sorted) {
        if (e.vibeQualityScore > bestQuality) {
            result.add(e)
            bestQuality = e.vibeQualityScore
        }
    }
    return result
}