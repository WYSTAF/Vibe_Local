package com.vibelocal.app.ui.chat

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Send
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.StopCircle
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.vibelocal.app.R
import com.vibelocal.app.core.ChatMessage
import com.vibelocal.app.core.ChatRoles
import com.vibelocal.app.core.LanWorkerController
import com.vibelocal.app.ui.common.ErrorBanner
import com.vibelocal.app.ui.common.InfoBanner
import com.vibelocal.app.ui.common.KeyValueRow
import com.vibelocal.app.ui.common.SectionCard
import com.vibelocal.app.ui.theme.AccentCyan
import com.vibelocal.app.util.formatBytes
import kotlin.math.roundToInt

@Composable
fun ChatScreen(vm: ChatViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val lan by vm.lanState.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current

    var input by rememberSaveable { mutableStateOf("") }
    val listState = rememberLazyListState()

    val notificationPermissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { _ ->
        // The worker can run even if notifications are denied (the ongoing
        // notification is simply hidden), so enable regardless of the result.
        vm.setLanEnabled(true)
    }
    val requestLanEnable: () -> Unit = {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
        } else {
            vm.setLanEnabled(true)
        }
    }

    LaunchedEffect(state.messages.size, state.sending) {
        if (state.messages.isNotEmpty()) {
            listState.animateScrollToItem(state.messages.size - 1)
        }
    }

    Column(modifier = Modifier.fillMaxSize()) {
        LazyColumn(
            state = listState,
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth(),
            contentPadding = PaddingValues(12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (state.messages.isEmpty()) {
                item {
                    Text(
                        text = stringResource(R.string.chat_empty),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(vertical = 16.dp),
                    )
                }
            }

            itemsIndexed(state.messages) { _, message ->
                MessageBubble(message)
            }

            if (state.sending) {
                item {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                        Text(
                            text = stringResource(R.string.common_loading),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }

            item { StatusCard(state) }
            item { LoadModelCard(state, vm) }
            item {
                LanWorkerCard(
                    lan = lan,
                    vm = vm,
                    onRequestEnable = requestLanEnable,
                    onCopy = { text ->
                        clipboard.setText(AnnotatedString(text))
                        Toast.makeText(context, R.string.common_copied, Toast.LENGTH_SHORT).show()
                    },
                )
            }
            item { SettingsCard(state, vm) }
            if (!vm.nativeAvailable) {
                item { RemoteHostCard(vm) }
            }
        }

        // Composer, pinned above the navigation bar and the IME.
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .imePadding()
                .padding(horizontal = 8.dp, vertical = 4.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Switch(checked = state.stripThink, onCheckedChange = { vm.toggleStripThink() })
                Spacer(Modifier.width(8.dp))
                Text(
                    text = stringResource(R.string.chat_strip_think),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedTextField(
                    value = input,
                    onValueChange = { input = it },
                    modifier = Modifier.weight(1f),
                    placeholder = { Text(stringResource(R.string.chat_hint)) },
                    maxLines = 4,
                    enabled = !state.sending,
                )
                FilledIconButton(
                    onClick = {
                        vm.send(input)
                        input = ""
                    },
                    enabled = input.isNotBlank() && !state.sending,
                ) {
                    Icon(
                        Icons.AutoMirrored.Outlined.Send,
                        contentDescription = stringResource(R.string.chat_send),
                    )
                }
            }
        }
    }
}

@Composable
private fun MessageBubble(message: ChatMessage) {
    val isUser = message.role == ChatRoles.USER
    val containerColor = when {
        message.isError -> MaterialTheme.colorScheme.errorContainer
        isUser -> MaterialTheme.colorScheme.primaryContainer
        else -> MaterialTheme.colorScheme.surfaceVariant
    }
    val contentColor = when {
        message.isError -> MaterialTheme.colorScheme.onErrorContainer
        isUser -> MaterialTheme.colorScheme.onPrimaryContainer
        else -> MaterialTheme.colorScheme.onSurface
    }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(14.dp),
        color = containerColor,
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Text(
                text = stringResource(if (isUser) R.string.chat_you else R.string.chat_assistant),
                style = MaterialTheme.typography.labelSmall,
                color = contentColor.copy(alpha = 0.7f),
            )
            Text(
                text = message.content,
                style = MaterialTheme.typography.bodyMedium,
                color = contentColor,
            )
        }
    }
}

@Composable
private fun StatusCard(state: ChatUiState) {
    SectionCard(title = stringResource(R.string.chat_status_title)) {
        val status = state.status
        KeyValueRow(
            stringResource(R.string.chat_loaded_model),
            status?.loadedModel ?: stringResource(R.string.chat_none_loaded),
        )
        KeyValueRow(
            stringResource(R.string.chat_backend),
            status?.backend ?: stringResource(R.string.common_not_available),
        )
        KeyValueRow(
            stringResource(R.string.chat_engine),
            status?.engine ?: stringResource(R.string.common_not_available),
        )
        val thermal = status?.thermal
        KeyValueRow(
            stringResource(R.string.chat_thermal),
            when {
                thermal?.celsius != null -> stringResource(R.string.units_celsius, thermal.celsius)
                thermal != null -> thermal.state
                else -> stringResource(R.string.common_not_available)
            },
        )
        KeyValueRow(
            stringResource(R.string.chat_idle_timeout),
            stringResource(R.string.settings_idle_timeout_value, state.idleTimeoutSecs),
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LoadModelCard(state: ChatUiState, vm: ChatViewModel) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    var nglText by rememberSaveable { mutableStateOf(state.ngl.toString()) }
    var ctxText by rememberSaveable { mutableStateOf(state.ctx.toString()) }

    SectionCard(title = stringResource(R.string.chat_load_title)) {
        val selectedName = state.models.firstOrNull { it.path == state.selectedPath }?.name
            ?: stringResource(R.string.chat_pick_model)

        ExposedDropdownMenuBox(
            expanded = expanded,
            onExpandedChange = { expanded = it },
        ) {
            OutlinedTextField(
                value = selectedName,
                onValueChange = {},
                readOnly = true,
                label = { Text(stringResource(R.string.chat_pick_model)) },
                trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded) },
                modifier = Modifier
                    .menuAnchor()
                    .fillMaxWidth(),
            )
            ExposedDropdownMenu(
                expanded = expanded,
                onDismissRequest = { expanded = false },
            ) {
                state.models.forEach { model ->
                    DropdownMenuItem(
                        text = { Text("${model.name} · ${formatBytes(model.sizeBytes)}") },
                        onClick = {
                            vm.selectModel(model.path)
                            expanded = false
                        },
                    )
                }
            }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = state.backendOverride,
                onValueChange = vm::setBackendOverride,
                label = { Text(stringResource(R.string.chat_backend_override)) },
                placeholder = { Text(stringResource(R.string.chat_backend_auto)) },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            OutlinedTextField(
                value = nglText,
                onValueChange = {
                    nglText = it
                    it.toIntOrNull()?.let(vm::setNgl)
                },
                label = { Text(stringResource(R.string.chat_ngl)) },
                singleLine = true,
                modifier = Modifier.weight(0.6f),
            )
            OutlinedTextField(
                value = ctxText,
                onValueChange = {
                    ctxText = it
                    it.toIntOrNull()?.let(vm::setCtx)
                },
                label = { Text(stringResource(R.string.chat_ctx)) },
                singleLine = true,
                modifier = Modifier.weight(0.8f),
            )
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = vm::loadSelected,
                enabled = !state.loadingModel && state.selectedPath != null,
                modifier = Modifier.weight(1f),
            ) {
                if (state.loadingModel) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary,
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.chat_loading_model))
                } else {
                    Text(stringResource(R.string.chat_load_button))
                }
            }
            OutlinedButton(
                onClick = vm::unload,
                enabled = state.status?.loadedModel != null,
                modifier = Modifier.weight(1f),
            ) {
                Text(stringResource(R.string.chat_unload_button))
            }
        }

        state.loadError?.let { ErrorBanner(it) }
    }
}

@Composable
private fun LanWorkerCard(
    lan: LanWorkerController.LanWorkerState,
    vm: ChatViewModel,
    onRequestEnable: () -> Unit,
    onCopy: (String) -> Unit,
) {
    SectionCard(title = stringResource(R.string.lan_title)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = stringResource(R.string.lan_toggle),
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text(
                    text = stringResource(
                        when {
                            lan.starting -> R.string.lan_starting
                            lan.enabled -> R.string.lan_running
                            else -> R.string.lan_stopped
                        },
                    ),
                    style = MaterialTheme.typography.labelSmall,
                    color = if (lan.enabled) AccentCyan else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(
                checked = lan.enabled || lan.starting,
                onCheckedChange = { want ->
                    if (want) onRequestEnable() else vm.setLanEnabled(false)
                },
                enabled = vm.nativeAvailable && !lan.starting,
            )
        }

        if (!vm.nativeAvailable) {
            InfoBanner(stringResource(R.string.lan_native_required))
        }
        lan.error?.let { ErrorBanner(it) }

        if (lan.enabled) {
            lan.token?.let { token ->
                Text(
                    text = stringResource(R.string.lan_token_label),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = token,
                        fontFamily = FontFamily.Monospace,
                        fontSize = 13.sp,
                        color = AccentCyan,
                        modifier = Modifier.weight(1f),
                    )
                    IconButton(onClick = { onCopy(token) }) {
                        Icon(
                            Icons.Outlined.ContentCopy,
                            contentDescription = stringResource(R.string.common_copy),
                            modifier = Modifier.size(18.dp),
                        )
                    }
                }
            }
            lan.ip?.let { ip ->
                KeyValueRow(
                    stringResource(R.string.lan_ip_label),
                    "$ip:${lan.port}",
                )
            }
            Text(
                text = stringResource(R.string.lan_pairing_hint),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            OutlinedButton(
                onClick = { vm.setLanEnabled(false) },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(Icons.Outlined.StopCircle, contentDescription = null, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text(stringResource(R.string.lan_stop))
            }
        }
    }
}

@Composable
private fun SettingsCard(state: ChatUiState, vm: ChatViewModel) {
    SectionCard(title = stringResource(R.string.settings_title)) {
        Text(
            text = stringResource(R.string.settings_idle_timeout) + ": " +
                stringResource(R.string.settings_idle_timeout_value, state.idleTimeoutSecs),
            style = MaterialTheme.typography.bodyMedium,
        )
        Slider(
            value = state.idleTimeoutSecs.toFloat(),
            onValueChange = { vm.setIdleTimeout(it.roundToInt()) },
            valueRange = 60f..1800f,
            steps = 28,
        )
    }
}

@Composable
private fun RemoteHostCard(vm: ChatViewModel) {
    val remoteHost by vm.remoteHost.collectAsStateWithLifecycle()
    var hostText by rememberSaveable(remoteHost) { mutableStateOf(remoteHost) }
    var tokenText by rememberSaveable { mutableStateOf(vm.remoteToken) }
    var message by rememberSaveable { mutableStateOf<String?>(null) }
    var messageIsError by rememberSaveable { mutableStateOf(false) }
    val invalidText = stringResource(R.string.settings_host_invalid)
    val savedText = stringResource(R.string.settings_saved)

    SectionCard(title = stringResource(R.string.settings_remote_host)) {
        OutlinedTextField(
            value = hostText,
            onValueChange = { hostText = it },
            label = { Text(stringResource(R.string.settings_remote_host)) },
            placeholder = { Text(stringResource(R.string.settings_remote_host_hint)) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = tokenText,
            onValueChange = { tokenText = it },
            label = { Text(stringResource(R.string.settings_remote_token)) },
            placeholder = { Text(stringResource(R.string.settings_remote_token_hint)) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Button(
            onClick = {
                val ok = vm.applyRemote(hostText, tokenText)
                messageIsError = !ok
                message = if (ok) savedText else invalidText
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(stringResource(R.string.common_apply))
        }
        message?.let {
            if (messageIsError) ErrorBanner(it) else InfoBanner(it)
        }
    }
}