package com.tsubuzaki.WoooshGo.settings

import android.content.Context
import android.os.Build
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map

enum class Visibility(val txtValue: String) {
    EVERYONE("e"),
    PAIRED_ONLY("p"),
    OFF(""),
}

/**
 * A relay only introduces two devices (DESIGN.md §9.1); file data always travels directly
 * and a transfer with no direct path is refused rather than relayed.
 */
enum class RelayMode {
    /** n0's free public relays. */
    PUBLIC,

    /** Advertised in this device's tickets. */
    CUSTOM,

    /** Wooosh contacts nothing and neither publishes nor redeems tickets. */
    OFF,
}

data class Settings(
    val displayName: String,
    val visibility: Visibility,
    val relayMode: RelayMode,
    /** Kept across mode changes so switching away and back does not lose what was typed. */
    val relayUrl: String,
) {
    val internetEnabled: Boolean
        get() = relayMode != RelayMode.OFF

    /** A blank CUSTOM address falls back to the public set, never to no path at all. */
    val relayUrls: List<String>?
        get() = when (relayMode) {
            RelayMode.PUBLIC -> null
            // Explicitly empty, not unset, so nothing that got past the UI reaches a relay.
            RelayMode.OFF -> emptyList()
            RelayMode.CUSTOM -> relayUrl.trim().takeIf { it.isNotEmpty() }?.let { listOf(it) }
        }
}

private val Context.dataStore by preferencesDataStore(name = "wooosh_settings")

class SettingsRepository(context: Context) {

    private val appContext = context.applicationContext

    val settings: Flow<Settings> = appContext.dataStore.data
        .map { prefs ->
            Settings(
                displayName = prefs[KEY_DISPLAY_NAME]?.takeIf { it.isNotBlank() } ?: Build.MODEL,
                // A fresh install must not accept transfers from strangers unopted-in.
                visibility = prefs[KEY_VISIBILITY]
                    ?.let { stored -> Visibility.entries.firstOrNull { it.name == stored } }
                    ?: Visibility.PAIRED_ONLY,
                relayMode = prefs[KEY_RELAY_MODE]
                    ?.let { stored -> RelayMode.entries.firstOrNull { it.name == stored } }
                    ?: RelayMode.PUBLIC,
                relayUrl = prefs[KEY_RELAY_URL].orEmpty(),
            )
        }
        .distinctUntilChanged()

    suspend fun setDisplayName(name: String) {
        appContext.dataStore.edit { it[KEY_DISPLAY_NAME] = name }
    }

    suspend fun setVisibility(visibility: Visibility) {
        appContext.dataStore.edit { it[KEY_VISIBILITY] = visibility.name }
    }

    suspend fun setRelayMode(mode: RelayMode) {
        appContext.dataStore.edit { it[KEY_RELAY_MODE] = mode.name }
    }

    suspend fun setRelayUrl(url: String) {
        appContext.dataStore.edit { it[KEY_RELAY_URL] = url }
    }

    private companion object {
        val KEY_DISPLAY_NAME = stringPreferencesKey("display_name")
        val KEY_VISIBILITY = stringPreferencesKey("visibility")
        val KEY_RELAY_MODE = stringPreferencesKey("relay_mode")
        val KEY_RELAY_URL = stringPreferencesKey("relay_url")
    }
}
