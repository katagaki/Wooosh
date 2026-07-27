package com.tsubuzaki.WoooshGo.ui

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DesktopWindows
import androidx.compose.material.icons.outlined.DeviceUnknown
import androidx.compose.material.icons.outlined.LaptopMac
import androidx.compose.material.icons.outlined.PhoneIphone
import androidx.compose.material.icons.outlined.Smartphone
import androidx.compose.material.icons.outlined.TabletAndroid
import androidx.compose.material.icons.outlined.TabletMac
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import com.tsubuzaki.WoooshGo.R
import com.tsubuzaki.WoooshGo.peers.DeviceType

/** UNKNOWN is a neutral glyph on purpose: never guess a platform (PROTOCOL.md §3.1). */
fun DeviceType.icon(): ImageVector = when (this) {
    DeviceType.IPHONE -> Icons.Outlined.PhoneIphone
    DeviceType.IPAD -> Icons.Outlined.TabletMac
    DeviceType.MAC -> Icons.Outlined.LaptopMac
    DeviceType.WINDOWS -> Icons.Outlined.DesktopWindows
    DeviceType.ANDROID_PHONE -> Icons.Outlined.Smartphone
    DeviceType.ANDROID_TABLET -> Icons.Outlined.TabletAndroid
    DeviceType.UNKNOWN -> Icons.Outlined.DeviceUnknown
}

@Composable
fun DeviceType.label(): String = stringResource(labelRes())

fun DeviceType.labelRes(): Int = when (this) {
    DeviceType.IPHONE -> R.string.device_kind_iphone
    DeviceType.IPAD -> R.string.device_kind_ipad
    DeviceType.MAC -> R.string.device_kind_mac
    DeviceType.WINDOWS -> R.string.device_kind_windows
    DeviceType.ANDROID_PHONE -> R.string.device_kind_android_phone
    DeviceType.ANDROID_TABLET -> R.string.device_kind_android_tablet
    DeviceType.UNKNOWN -> R.string.device_kind_unknown
}
