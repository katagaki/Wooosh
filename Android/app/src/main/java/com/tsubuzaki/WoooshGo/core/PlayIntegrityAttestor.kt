package com.tsubuzaki.WoooshGo.core

import android.content.Context
import android.util.Log
import com.google.android.gms.tasks.Tasks
import com.google.android.play.core.integrity.IntegrityManagerFactory
import com.google.android.play.core.integrity.StandardIntegrityManager.PrepareIntegrityTokenRequest
import com.google.android.play.core.integrity.StandardIntegrityManager.StandardIntegrityTokenProvider
import com.google.android.play.core.integrity.StandardIntegrityManager.StandardIntegrityTokenRequest
import com.tsubuzaki.WoooshGo.BuildConfig
import java.util.concurrent.TimeUnit
import uniffi.wooosh_core.AppAttestEvidence
import uniffi.wooosh_core.AttestationKind
import uniffi.wooosh_core.Attestor
import uniffi.wooosh_core.TpmEvidence

/**
 * Play Integrity for rendezvous admission (PROTOCOL.md §9.2). The core calls this on its
 * own blocking threads, never the main thread, so the Play tasks are awaited directly.
 * Any failure returns null, which admits the device as Unverified rather than failing.
 */
class PlayIntegrityAttestor(context: Context) : Attestor {
    private val appContext = context.applicationContext
    private val projectNumber = BuildConfig.PLAY_CLOUD_PROJECT_NUMBER

    /** Prepared once and reused; Google recommends warming it ahead of requests. */
    @Volatile
    private var provider: StandardIntegrityTokenProvider? = null

    override fun kind(): AttestationKind =
        if (projectNumber != 0L) AttestationKind.PLAY_INTEGRITY else AttestationKind.NONE

    override fun playIntegrity(requestHash: String): String? {
        if (projectNumber == 0L) return null
        return runCatching { request(requestHash) }
            .recoverCatching {
                // A stale provider fails every request until it is prepared again.
                provider = null
                request(requestHash)
            }
            .onFailure { Log.w(TAG, "Play Integrity unavailable", it) }
            .getOrNull()
    }

    private fun request(requestHash: String): String {
        val p = provider ?: Tasks.await(
            IntegrityManagerFactory.createStandard(appContext).prepareIntegrityToken(
                PrepareIntegrityTokenRequest.builder().setCloudProjectNumber(projectNumber).build()
            ),
            TIMEOUT_SECONDS,
            TimeUnit.SECONDS,
        ).also { provider = it }
        return Tasks.await(
            p.request(StandardIntegrityTokenRequest.builder().setRequestHash(requestHash).build()),
            TIMEOUT_SECONDS,
            TimeUnit.SECONDS,
        ).token()
    }

    override fun appAttest(clientDataHash: ByteArray, attest: Boolean): AppAttestEvidence? = null

    override fun tpmEvidence(): TpmEvidence? = null

    override fun tpmActivate(credentialBlob: ByteArray, encryptedSecret: ByteArray): ByteArray? = null

    private companion object {
        const val TAG = "PlayIntegrity"
        const val TIMEOUT_SECONDS = 30L
    }
}
