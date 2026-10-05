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

/** Which rendezvous service introduces two devices over the internet (DESIGN.md §9.1). */
enum class InternetMode {
    /** The Wooosh Worker this build was compiled against. */
    WOOOSH,

    /** A self-deployed copy of the Worker. Both devices must use it. */
    CUSTOM,

    /** Wooosh contacts nothing and neither publishes nor redeems codes. */
    OFF,
}

data class Settings(
    val displayName: String,
    val visibility: Visibility,
    val internetMode: InternetMode,
    /** Kept across mode changes so switching away and back does not lose what was typed. */
    val internetServerUrl: String,
) {
    val internetEnabled: Boolean
        get() = internetMode != InternetMode.OFF

    /** A blank CUSTOM address falls back to the build's service, never to no path at all. */
    val rendezvousUrl: String?
        get() = when (internetMode) {
            InternetMode.WOOOSH -> null
            // Explicitly empty, not unset, so nothing that got past the UI reaches a server.
            InternetMode.OFF -> ""
            InternetMode.CUSTOM -> internetServerUrl.trim().takeIf { it.isNotEmpty() }
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
                internetMode = prefs[KEY_INTERNET_MODE]
                    ?.let { stored -> InternetMode.entries.firstOrNull { it.name == stored } }
                    // Only "off" carries over from the iroh-era setting: a relay is no server.
                    ?: if (prefs[KEY_LEGACY_RELAY_MODE] == "OFF") InternetMode.OFF else InternetMode.WOOOSH,
                internetServerUrl = prefs[KEY_INTERNET_SERVER_URL].orEmpty(),
            )
        }
        .distinctUntilChanged()

    suspend fun setDisplayName(name: String) {
        appContext.dataStore.edit { it[KEY_DISPLAY_NAME] = name }
    }

    suspend fun setVisibility(visibility: Visibility) {
        appContext.dataStore.edit { it[KEY_VISIBILITY] = visibility.name }
    }

    suspend fun setInternetMode(mode: InternetMode) {
        appContext.dataStore.edit { it[KEY_INTERNET_MODE] = mode.name }
    }

    suspend fun setInternetServerUrl(url: String) {
        appContext.dataStore.edit { it[KEY_INTERNET_SERVER_URL] = url }
    }

    private companion object {
        val KEY_DISPLAY_NAME = stringPreferencesKey("display_name")
        val KEY_VISIBILITY = stringPreferencesKey("visibility")
        val KEY_INTERNET_MODE = stringPreferencesKey("internet_mode")
        val KEY_INTERNET_SERVER_URL = stringPreferencesKey("internet_server_url")
        val KEY_LEGACY_RELAY_MODE = stringPreferencesKey("relay_mode")
    }
}
