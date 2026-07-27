package com.tsubuzaki.WoooshGo.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.pairing.PairingManager
import com.tsubuzaki.WoooshGo.pairing.PairingManager.AttemptState

/** Modal on purpose, but must always offer a way out (PROTOCOL.md §4.2 / §4.3). */
@Composable
fun PairingProgressDialog(
    attempt: PairingManager.Attempt,
    onCancel: () -> Unit,
    onDismiss: () -> Unit,
) {
    when (attempt.state) {
        AttemptState.CONNECTING -> AlertDialog(
            // Back cancels the wait rather than hiding an inescapable spinner.
            onDismissRequest = onCancel,
            properties = DialogProperties(
                dismissOnBackPress = true,
                dismissOnClickOutside = false,
            ),
            icon = {
                CircularProgressIndicator(
                    modifier = Modifier.size(28.dp),
                    strokeWidth = 3.dp,
                )
            },
            title = {
                Text(
                    stringResource(
                        if (attempt.isTicket) R.string.internet_connecting_title
                        else R.string.pairing_in_progress_title
                    )
                )
            },
            text = {
                Column {
                    Text(stringResource(R.string.transfer_connecting_to, attempt.deviceName))
                    Spacer(Modifier.height(8.dp))
                    Text(
                        text = stringResource(
                            if (attempt.isTicket) R.string.internet_progress_body
                            else R.string.pairing_progress_body
                        ),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
            confirmButton = {},
            dismissButton = {
                TextButton(onClick = onCancel) { Text(stringResource(R.string.action_cancel)) }
            },
        )

        AttemptState.FAILED -> AlertDialog(
            onDismissRequest = onDismiss,
            icon = {
                Icon(
                    Icons.Outlined.ErrorOutline,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.error,
                )
            },
            title = {
                Text(
                    stringResource(
                        if (attempt.isTicket) R.string.internet_failed_title_named
                        else R.string.pairing_failed_title_named,
                        attempt.deviceName,
                    )
                )
            },
            text = { Text(attempt.message ?: stringResource(R.string.error_pairing_failed)) },
            confirmButton = {
                Button(onClick = onDismiss) { Text(stringResource(R.string.action_ok)) }
            },
        )

        AttemptState.SUCCEEDED -> AlertDialog(
            onDismissRequest = onDismiss,
            icon = {
                Icon(
                    Icons.Outlined.CheckCircle,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                )
            },
            title = { Text(stringResource(R.string.pairing_success_title, attempt.deviceName)) },
            text = { Text(stringResource(R.string.pairing_success_body)) },
            confirmButton = {
                Button(onClick = onDismiss) { Text(stringResource(R.string.action_done)) }
            },
        )
    }
}
