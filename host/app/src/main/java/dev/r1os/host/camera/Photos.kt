package dev.r1os.host.camera

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import android.widget.ImageView
import dev.r1os.host.net.R1d
import org.json.JSONObject

/**
 * Taking photos and showing them (ADR-0031): a still from [Camera] goes to r1d's media store
 * (`POST /api/media`), and thumbnails come back from it, decoded small and kept a while.
 */
object Photos {
    /** Decoded thumbnails, by media id and size. */
    private val cache = object : LruCache<String, Bitmap>(12 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount
    }

    /** Takes a photo and keeps it: `source` says where from (`camera`, `chat`, `look`). Its media id. */
    fun take(context: Context, source: String, onDone: (String) -> Unit, onError: (String) -> Unit) = take(context, source, null, onDone, onError)

    /** Taken through `film` (the camera app's); `stream`: its preview's frame at the shutter ([Camera.capture]). */
    fun take(context: Context, source: String, film: Film?, onDone: (String) -> Unit, onError: (String) -> Unit, stream: Bitmap? = null) {
        Camera.capture(context, film, onJpeg = { jpeg ->
            R1d.upload("/api/media?source=${R1d.enc(source)}", jpeg, "image/jpeg", onError = onError) { r ->
                val id = r.optJSONObject("media")?.optString("id").orEmpty()
                if (id.isEmpty()) onError("The photo wasn't kept.") else onDone(id)
            }
        }, onError = onError, stream = stream)
    }

    /** Shows photo `id` in `view`, about `px` on its long side. */
    fun show(view: ImageView, id: String, px: Int) {
        val key = "$id@$px"
        view.tag = key
        cache.get(key)?.let { view.setImageBitmap(it); return }
        view.setImageDrawable(null)
        R1d.bytes("/api/media/${R1d.enc(id)}/file") { bytes ->
            if (bytes == null) return@bytes
            val bmp = decode(bytes, px) ?: return@bytes
            cache.put(key, bmp)
            if (view.tag == key) view.setImageBitmap(bmp)
        }
    }

    /** Decoded at about `px` on its long side (sampled down, so a big photo costs little memory). */
    private fun decode(bytes: ByteArray, px: Int): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        var sample = 1
        while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= px) sample *= 2
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
    }

    /** A media item's ids from a message's meta (`images`). */
    fun idsOf(meta: JSONObject?): List<String> {
        val a = meta?.optJSONArray("images") ?: return emptyList()
        return (0 until a.length()).map { a.optString(it) }.filter { it.isNotEmpty() }
    }
}
