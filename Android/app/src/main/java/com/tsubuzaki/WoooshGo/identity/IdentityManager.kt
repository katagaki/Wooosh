package com.tsubuzaki.WoooshGo.identity

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import androidx.core.content.edit
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.wooosh_core.KeyStore as CoreKeyStore

/**
 * Platform key-storage adapter for the Rust core (DESIGN.md §4 `PlatformAdapters.key_store`,
 * PROTOCOL.md §2).
 *
 * The core owns the identity: it calls [loadIdentity] on start and, only on first launch,
 * generates an Ed25519 keypair and hands the 32-byte secret back through [storeIdentity].
 * The shell never derives a public key or a DeviceID of its own — `core.deviceId()` and
 * `core.fingerprintPhrase()` are the single source of truth.
 *
 * Android Keystore has no Curve25519 below API 33, so the 32-byte secret lives in
 * SharedPreferences encrypted with AES-GCM under a Keystore-held AES key. This layout is
 * load-bearing for upgrades: changing it costs every existing install its identity.
 */
class IdentityManager(context: Context) : CoreKeyStore {

    private val appContext = context.applicationContext
    private val lock = Any()

    private val prefs
        get() = appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /** Called by the core on start. Null on a genuinely fresh install. */
    override fun loadIdentity(): ByteArray? {
        synchronized(lock) {
            val storedCiphertext = prefs.getString(KEY_SEED_CIPHERTEXT, null) ?: return null
            val storedIv = prefs.getString(KEY_SEED_IV, null) ?: return null
            val secret = runCatching {
                decrypt(
                    Base64.decode(storedCiphertext, Base64.NO_WRAP),
                    Base64.decode(storedIv, Base64.NO_WRAP),
                )
            }.getOrElse { error ->
                // Unwrapping failed (Keystore key lost — e.g. after a device restore).
                // Drop the unusable blob so the core mints a fresh identity instead of
                // failing to start; the user re-pairs, which is the honest outcome.
                Log.w(TAG, "identity unwrap failed, discarding stored key", error)
                prefs.edit { remove(KEY_SEED_CIPHERTEXT).remove(KEY_SEED_IV) }
                return null
            }
            if (secret.size != SEED_SIZE_BYTES) {
                Log.w(TAG, "stored identity is ${secret.size} bytes, expected $SEED_SIZE_BYTES")
                prefs.edit { remove(KEY_SEED_CIPHERTEXT).remove(KEY_SEED_IV) }
                return null
            }
            Log.i(TAG, "loaded existing identity key for the core")
            return secret
        }
    }

    /** Called by the core exactly once, on first launch. */
    override fun storeIdentity(secret: ByteArray) {
        synchronized(lock) {
            val (ciphertext, iv) = encrypt(secret)
            prefs.edit {
                putString(KEY_SEED_CIPHERTEXT, Base64.encodeToString(ciphertext, Base64.NO_WRAP))
                putString(KEY_SEED_IV, Base64.encodeToString(iv, Base64.NO_WRAP))
            }
            Log.i(TAG, "stored a new core-generated identity key (${secret.size} bytes)")
        }
    }

    private fun encrypt(plaintext: ByteArray): Pair<ByteArray, ByteArray> {
        val cipher = Cipher.getInstance(CIPHER_TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, keystoreKey())
        return cipher.doFinal(plaintext) to cipher.iv
    }

    private fun decrypt(ciphertext: ByteArray, iv: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(CIPHER_TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, keystoreKey(), GCMParameterSpec(GCM_TAG_BITS, iv))
        return cipher.doFinal(ciphertext)
    }

    private fun keystoreKey(): SecretKey {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (keyStore.getEntry(KEYSTORE_ALIAS, null) as? KeyStore.SecretKeyEntry)
            ?.let { return it.secretKey }

        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(
                KEYSTORE_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return generator.generateKey()
    }

    private companion object {
        const val TAG = "WoooshIdentity"
        const val PREFS_NAME = "wooosh_identity"
        const val KEY_SEED_CIPHERTEXT = "seed_ciphertext"
        const val KEY_SEED_IV = "seed_iv"
        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val KEYSTORE_ALIAS = "wooosh_identity_kek"
        const val CIPHER_TRANSFORMATION = "AES/GCM/NoPadding"
        const val GCM_TAG_BITS = 128
        const val SEED_SIZE_BYTES = 32
    }
}
