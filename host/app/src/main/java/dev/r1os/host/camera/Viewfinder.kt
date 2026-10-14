package dev.r1os.host.camera

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.graphics.SurfaceTexture
import android.view.TextureView
import android.view.View
import android.widget.FrameLayout
import dev.r1os.host.net.R1d

/**
 * A live view of the camera ([Camera]), filling its space, with a white flash for the shutter. It opens the
 * camera when its surface appears and lets go of it when it goes; [pause] and [resume] while it's covered.
 * [film] shows the camera app's film live: its colour on the preview's layer, its vignette and grain over it.
 */
@SuppressLint("ViewConstructor")
class Viewfinder(context: Context) : FrameLayout(context), TextureView.SurfaceTextureListener {
    private val texture = TextureView(context)
    private val flashView = View(context).apply {
        setBackgroundColor(Color.WHITE)
        alpha = 0f
    }
    private val grainView = object : View(context) {
        override fun onDraw(c: android.graphics.Canvas) = film.overlay(c, width.toFloat(), height.toFloat(), 1f)
    }
    private var film: Film = Film.STD
    private var stops = 0f

    /** The film, and exposure the camera can't do itself (stops), live on the preview. */
    fun develop(f: Film, gain: Float) {
        if (f == film && gain == stops) return
        film = f
        stops = gain
        texture.setLayerPaint(f.paint(gain))
        grainView.invalidate()
    }
    private var surface: SurfaceTexture? = null
    private var paused = false
    var onReady: () -> Unit = {}
    var onError: (String) -> Unit = {}

    init {
        setBackgroundColor(Color.BLACK)
        addView(texture, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        addView(grainView, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        addView(flashView, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        texture.surfaceTextureListener = this
    }

    override fun onSurfaceTextureAvailable(st: SurfaceTexture, w: Int, h: Int) {
        surface = st
        if (!paused) start()
    }

    private fun start() {
        val st = surface ?: return
        Camera.open(context, st, onReady = { fit(); onReady() }, onError = { onError(it) })
    }

    override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, w: Int, h: Int) = fit()

    override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
        surface = null
        Camera.setPreview(null)
        return true
    }

    override fun onSurfaceTextureUpdated(st: SurfaceTexture) {}

    /** The preview upright and filling the view (again after the camera turned around). */
    fun fit() {
        R1d.main.post { texture.setTransform(Camera.previewTransform(texture.width, texture.height)) }
    }

    /** Covered, or the display went off: the camera closes. */
    fun pause() {
        paused = true
        Camera.close()
    }

    fun resume() {
        if (!paused) return
        paused = false
        start()
    }

    /** What the preview shows now, `w` × `h` (a QR to read); null before it's up. */
    fun frame(w: Int, h: Int): android.graphics.Bitmap? = if (texture.isAvailable) texture.getBitmap(w, h) else null

    fun flash() {
        flashView.animate().cancel()
        flashView.alpha = 0.85f
        flashView.animate().alpha(0f).setDuration(280).start()
    }
}
