package com.tsubuzaki.WoooshGo.share

import android.net.Uri
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Holds files staged by the share-target entry point until the user picks a device in
 * the (armed) device list. In-memory by design: staged copies live in cacheDir and a
 * fresh share always re-arms it.
 */
class OutboxRepository {

    data class StagedShare(
        val uris: List<Uri>,
        /** Set when the share came in via a Direct Share shortcut for a paired device. */
        val targetDeviceId: String? = null,
        val targetDisplayName: String? = null,
    )

    private val _staged = MutableStateFlow<StagedShare?>(null)
    val staged: StateFlow<StagedShare?> = _staged.asStateFlow()

    fun arm(share: StagedShare) {
        _staged.value = share
    }

    fun clear() {
        _staged.value = null
    }
}
