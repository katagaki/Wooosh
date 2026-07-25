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
 * Which relays the internet path may use (DESIGN.md §9.1).
 *
 * A relay only ever introduces two devices; file data always travels directly, and a
 * transfer that cannot find a direct path is refused rather than relayed. So this picks
 * who helps the two devices meet, not who carries the files.
 */
enum class RelayMode {
    /** n0's free public relays, shared with every other iroh user. */
    PUBLIC,

    /** A relay the user runs or chose. Tickets from this device advertise it. */
    CUSTOM,

    /**
     * Internet transfers are off. Wooosh contacts nothing and neither publishes nor
     * redeems tickets; only devices on the same network.
     */
    OFF,
}

data class Settings(
    val displayName: String,
    val visibility: Visibility,
    val relayMode: RelayMode,
    /**
     * Only meaningful for [RelayMode.CUSTOM]; kept across mode changes so switching away
     * and back does not lose what was typed.
     */
    val relayUrl: String,
) {
    /**
     * Whether the internet path is available at all. When false the UI neither publishes
     * nor redeems tickets, so nothing can bind an endpoint.
     */
    val internetEnabled: Boolean
        get() = relayMode != RelayMode.OFF

    /**
     * The value `setRelayUrls` takes: null for the public set, empty for no relays at
     * all. A CUSTOM mode with a blank address is not a valid configuration, so it falls
     * back to the public set rather than silently disabling the internet path.
     */
    val relayUrls: List<String>?
        get() = when (relayMode) {
            RelayMode.PUBLIC -> null
            // OFF still resolves to "no relays" rather than being left unset: the UI is
            // what stops a ticket being made, and this makes sure a code path that got
            // past it cannot reach a relay either.
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
                visibility = prefs[KEY_VISIBILITY]
                    ?.let { stored -> Visibility.entries.firstOrNull { it.name == stored } }
                    ?: Visibility.EVERYONE,
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
