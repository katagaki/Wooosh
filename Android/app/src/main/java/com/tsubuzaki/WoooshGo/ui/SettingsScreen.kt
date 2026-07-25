package com.tsubuzaki.WoooshGo.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.material3.TextButton
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.settings.Settings
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import com.tsubuzaki.WoooshGo.settings.RelayMode
import com.tsubuzaki.WoooshGo.settings.Visibility
import com.tsubuzaki.WoooshGo.core.TrustedPeerInfo
import java.text.DateFormat
import java.util.Date

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    settings: Settings,
    deviceIdFormatted: String?,
    fingerprintPhrase: String?,
    pairedDevices: List<TrustedPeerInfo>,
    onRevokeDevice: (String) -> Unit,
    onDisplayNameChange: (String) -> Unit,
    onVisibilityChange: (Visibility) -> Unit,
    onRelayModeChange: (RelayMode) -> Unit,
    onRelayUrlChange: (String) -> Unit,
    relayError: String?,
    onBack: () -> Unit,
) {
    // Local edit buffer so persistence (and the discovery re-registration debounce)
    // doesn't fight the text field cursor.
    var displayName by rememberSaveable { mutableStateOf(settings.displayName) }
    var relayUrl by rememberSaveable { mutableStateOf(settings.relayUrl) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.action_settings)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Outlined.ArrowBack,
                            contentDescription = stringResource(R.string.action_back),
                        )
                    }
                },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
        ) {
            OutlinedTextField(
                value = displayName,
                onValueChange = {
                    displayName = it
                    onDisplayNameChange(it)
                },
                label = { Text(stringResource(R.string.settings_display_name)) },
                singleLine = true,
                supportingText = { Text(stringResource(R.string.settings_display_name_footer)) },
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(Modifier.height(24.dp))

            Text(
                text = stringResource(R.string.settings_section_who_can_see),
                style = MaterialTheme.typography.titleMedium,
            )
            Spacer(Modifier.height(8.dp))
            Column(Modifier.selectableGroup()) {
                VisibilityOption(
                    label = stringResource(R.string.settings_visibility_everyone),
                    description = stringResource(R.string.settings_visibility_everyone_desc),
                    selected = settings.visibility == Visibility.EVERYONE,
                    onClick = { onVisibilityChange(Visibility.EVERYONE) },
                )
                VisibilityOption(
                    label = stringResource(R.string.settings_visibility_paired),
                    description = stringResource(R.string.settings_visibility_paired_desc),
                    selected = settings.visibility == Visibility.PAIRED_ONLY,
                    onClick = { onVisibilityChange(Visibility.PAIRED_ONLY) },
                )
                VisibilityOption(
                    label = stringResource(R.string.settings_visibility_off),
                    description = stringResource(R.string.settings_visibility_off_desc),
                    selected = settings.visibility == Visibility.OFF,
                    onClick = { onVisibilityChange(Visibility.OFF) },
                )
            }

            Spacer(Modifier.height(24.dp))
            HorizontalDivider()
            Spacer(Modifier.height(24.dp))

            // Worth showing rather than hiding behind a default: the three options
            // differ in who has to be available for an internet transfer to work, and
            // this is the only place Wooosh says a relay never carries file data.
            Text(
                text = stringResource(R.string.settings_section_relay),
                style = MaterialTheme.typography.titleMedium,
            )
            Spacer(Modifier.height(8.dp))
            Column(Modifier.selectableGroup()) {
                VisibilityOption(
                    label = stringResource(R.string.settings_relay_public),
                    description = stringResource(R.string.settings_relay_public_footer),
                    selected = settings.relayMode == RelayMode.PUBLIC,
                    onClick = { onRelayModeChange(RelayMode.PUBLIC) },
                )
                VisibilityOption(
                    label = stringResource(R.string.settings_relay_custom),
                    description = stringResource(R.string.settings_relay_custom_footer),
                    selected = settings.relayMode == RelayMode.CUSTOM,
                    onClick = { onRelayModeChange(RelayMode.CUSTOM) },
                )
                VisibilityOption(
                    label = stringResource(R.string.settings_relay_off),
                    description = stringResource(R.string.settings_relay_off_footer),
                    selected = settings.relayMode == RelayMode.OFF,
                    onClick = { onRelayModeChange(RelayMode.OFF) },
                )
            }
            if (settings.relayMode == RelayMode.CUSTOM) {
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = relayUrl,
                    onValueChange = {
                        relayUrl = it
                        onRelayUrlChange(it)
                    },
                    label = { Text(stringResource(R.string.settings_relay_url)) },
                    singleLine = true,
                    isError = relayError != null,
                    supportingText = relayError?.let { { Text(it) } },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    modifier = Modifier.fillMaxWidth(),
                )
            }

            Spacer(Modifier.height(24.dp))
            HorizontalDivider()
            Spacer(Modifier.height(24.dp))

            Text(
                text = stringResource(R.string.settings_section_paired_devices),
                style = MaterialTheme.typography.titleMedium,
            )
            Spacer(Modifier.height(8.dp))
            if (pairedDevices.isEmpty()) {
                Text(
                    text = stringResource(R.string.settings_paired_devices_empty),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                // Straight from the core's trust store: DeviceID is the identity and the
                // display name is only a label.
                pairedDevices.forEach { device ->
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 4.dp),
                    ) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                text = device.displayName,
                                style = MaterialTheme.typography.bodyLarge,
                            )
                            Text(
                                text = device.deviceId,
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text(
                                text = device.fingerprint,
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text(
                                text = stringResource(
                                    R.string.settings_paired_on,
                                    DateFormat.getDateInstance(DateFormat.MEDIUM)
                                        .format(Date(device.pairedAtMillis)),
                                ),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        TextButton(onClick = { onRevokeDevice(device.deviceId) }) {
                            Text(
                                stringResource(R.string.action_unpair),
                                color = MaterialTheme.colorScheme.error,
                            )
                        }
                    }
                }
            }

            Spacer(Modifier.height(24.dp))
            HorizontalDivider()
            Spacer(Modifier.height(24.dp))

            Text(
                text = stringResource(R.string.settings_device_id),
                style = MaterialTheme.typography.titleMedium,
            )
            Spacer(Modifier.height(8.dp))
            // Straight from the core — BLAKE3(pubkey)[0..16] (PROTOCOL.md §2).
            Text(
                text = deviceIdFormatted ?: stringResource(R.string.settings_starting),
                style = MaterialTheme.typography.bodyLarge,
                fontFamily = FontFamily.Monospace,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.settings_device_id_footer),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Spacer(Modifier.height(16.dp))
            Text(
                text = stringResource(R.string.verify_phrase_title),
                style = MaterialTheme.typography.titleMedium,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = fingerprintPhrase ?: stringResource(R.string.settings_starting),
                style = MaterialTheme.typography.bodyLarge,
                fontFamily = FontFamily.Monospace,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.verify_phrase_body),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun VisibilityOption(
    label: String,
    description: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .selectable(selected = selected, onClick = onClick, role = Role.RadioButton)
            .padding(vertical = 8.dp),
    ) {
        RadioButton(selected = selected, onClick = null)
        Spacer(Modifier.width(12.dp))
        Column {
            Text(text = label, style = MaterialTheme.typography.bodyLarge)
            Text(
                text = description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
