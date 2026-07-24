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

data class Settings(
    val displayName: String,
    val visibility: Visibility,
)

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
            )
        }
        .distinctUntilChanged()

    suspend fun setDisplayName(name: String) {
        appContext.dataStore.edit { it[KEY_DISPLAY_NAME] = name }
    }

    suspend fun setVisibility(visibility: Visibility) {
        appContext.dataStore.edit { it[KEY_VISIBILITY] = visibility.name }
    }

    private companion object {
        val KEY_DISPLAY_NAME = stringPreferencesKey("display_name")
        val KEY_VISIBILITY = stringPreferencesKey("visibility")
    }
}
