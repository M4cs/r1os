package dev.r1os.host.camera

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageFormat
import android.graphics.Matrix
import android.graphics.Rect
import android.graphics.SurfaceTexture
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.SessionConfiguration
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.util.Log
import android.util.Range
import android.util.Rational
import android.util.Size
import android.view.Surface
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executor

/**
 * The R1's camera (ADR-0031), through Camera2. It's on the rotating module the motor turns (r1d's
 * `/api/motor`): toward the world, or around to you.
 *
 * One user at a time: a viewfinder (the Camera app, a chat's camera, push-to-talk's "what am I looking at"),
 * or nobody watching (System One opening it in the background while you talk, so the shot is instant when
 * you let go). Open, it keeps a preview running so exposure and focus are ready; [capture] takes a still,
 * turned upright and sized for an agent (at most [MAX_SIDE] px). [close] when done: an open camera costs
 * battery, so it closes itself after [IDLE_MS] unused, too.
 *
 * The camera app's dials (ADR-0046) are [controls]: exposure, white balance and zoom, on the preview and the still
 * alike. It puts them back to neutral when it leaves, so nothing else sees them.
 */
@SuppressLint("MissingPermission")
object Camera {
    private const val TAG = "r1os"
    /** A photo's long side at most (plenty for an agent to read a label; ~300 KB). */
    const val MAX_SIDE = 1600
    /** Opened with nobody using it: closed after this long. */
    private const val IDLE_MS = 15_000L
    /** A still that takes longer than this is an error (one cut short by the camera closing never reports). */
    private const val CAPTURE_TIMEOUT_MS = 6_000L

    private val main = Handler(Looper.getMainLooper())
    private var thread: HandlerThread? = null
    private var cam: Handler? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var reader: ImageReader? = null
    private var previewSurface: Surface? = null
    private var offscreen: SurfaceTexture? = null
    private var sensor = 90
    var previewSize = Size(640, 480)
        private set
    /** The module faces you (the motor turned it around): the picture is upside down then. */
    var facingUser = false
    /** Facing you, the preview (and the camera app's photos) as in a mirror (its settings; on at first). */
    var mirrorSelfies = true
    private var opening = false
    private val readyWaiters = ArrayList<() -> Unit>()
    private val closeIdle = Runnable { close() }

    /**
     * Exposure (in the camera's compensation steps, when it has them), white balance (a Camera2 mode), zoom (1 =
     * none), and exposure it can't do itself, in stops: a gain on the picture ([Film]).
     */
    data class Controls(val ev: Int = 0, val awb: Int = CaptureRequest.CONTROL_AWB_MODE_AUTO, val zoom: Float = 1f, val gain: Float = 0f)

    /** What this camera offers, once it has been opened. */
    class Caps(val ev: Range<Int>, val evStep: Rational, val awb: IntArray, val maxZoom: Float, val active: Rect)

    var controls = Controls()
        private set
    var caps: Caps? = null
        private set

    /** The dials changed: the preview shows them now, and the next still has them. */
    fun setControls(c: Controls) {
        controls = c
        cam?.post { repeat() }
    }

    val isOpen get() = session != null

    /**
     * Opens the camera (if it isn't) with `texture` as its preview, or an off-screen one. `onReady` once a
     * still can be taken.
     */
    fun open(context: Context, texture: SurfaceTexture?, onReady: () -> Unit = {}, onError: (String) -> Unit = {}) {
        main.removeCallbacks(closeIdle)
        if (texture == null) main.postDelayed(closeIdle, IDLE_MS)
        val handler = handler()
        handler.post {
            readyWaiters += { main.post(onReady) }
            try {
                if (device == null) {
                    if (opening) return@post
                    opening = true
                    val cm = context.getSystemService(CameraManager::class.java)
                    val id = cm.cameraIdList.firstOrNull() ?: throw IllegalStateException("no camera")
                    val ch = cm.getCameraCharacteristics(id)
                    sensor = ch.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
                    caps = Caps(
                        ch.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE) ?: Range(0, 0),
                        ch.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP) ?: Rational(1, 3),
                        ch.get(CameraCharacteristics.CONTROL_AWB_AVAILABLE_MODES) ?: intArrayOf(CaptureRequest.CONTROL_AWB_MODE_AUTO),
                        ch.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM) ?: 1f,
                        ch.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE) ?: Rect(0, 0, 3264, 2448),
                    )
                    val map = ch.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
                    val jpeg = map?.getOutputSizes(ImageFormat.JPEG).orEmpty()
                        .filter { maxOf(it.width, it.height) <= MAX_SIDE * 2 }
                        .maxByOrNull { it.width.toLong() * it.height } ?: Size(1280, 960)
                    // A 4:3 preview fills the R1's 3:4 screen once it's turned upright.
                    previewSize = map?.getOutputSizes(SurfaceTexture::class.java).orEmpty()
                        .filter { it.width * 3 == it.height * 4 && it.width <= 1280 }
                        .maxByOrNull { it.width } ?: Size(640, 480)
                    reader = ImageReader.newInstance(jpeg.width, jpeg.height, ImageFormat.JPEG, 2)
                    cm.openCamera(id, object : CameraDevice.StateCallback() {
                        override fun onOpened(d: CameraDevice) {
                            opening = false
                            device = d
                            startSession(texture)
                            // Rabbit's HAL turns the module to face out as it opens: once that's done, back to ours.
                            main.postDelayed({ Motor.cameraOpened() }, Motor.HAL_SETTLE_MS)
                        }
                        override fun onDisconnected(d: CameraDevice) = closeNow()
                        override fun onError(d: CameraDevice, error: Int) {
                            Log.w(TAG, "camera: error $error")
                            closeNow()
                            main.post { onError("The camera isn't working ($error).") }
                        }
                    }, handler)
                } else {
                    startSession(texture)
                }
            } catch (e: Exception) {
                opening = false
                Log.w(TAG, "camera: open", e)
                readyWaiters.clear()
                main.post { onError(e.message ?: "The camera isn't working.") }
            }
        }
    }

    /** (On the camera thread.) A session with the preview (on `texture`, or off screen) and the still reader. */
    private fun startSession(texture: SurfaceTexture?) {
        val d = device ?: return
        session?.close()
        session = null
        previewSurface?.release()
        val st = texture ?: (offscreen ?: SurfaceTexture(false).also { offscreen = it })
        st.setDefaultBufferSize(previewSize.width, previewSize.height)
        val preview = Surface(st)
        previewSurface = preview
        val still = reader?.surface ?: return
        val executor = Executor { cam?.post(it) }
        val config = SessionConfiguration(
            SessionConfiguration.SESSION_REGULAR,
            listOf(OutputConfiguration(preview), OutputConfiguration(still)),
            executor,
            object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(s: CameraCaptureSession) {
                    session = s
                    repeat()
                    val waiting = ArrayList(readyWaiters)
                    readyWaiters.clear()
                    waiting.forEach { it() }
                }
                override fun onConfigureFailed(s: CameraCaptureSession) {
                    Log.w(TAG, "camera: session failed")
                    readyWaiters.clear()
                }
            },
        )
        d.createCaptureSession(config)
    }

    /** (On the camera thread.) The preview's repeating request, with the dials. */
    private fun repeat() {
        val d = device ?: return
        val s = session ?: return
        val preview = previewSurface ?: return
        runCatching {
            val req = d.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
                addTarget(preview)
                set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO)
                set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                dials(this)
            }
            s.setRepeatingRequest(req.build(), null, cam)
        }.onFailure { Log.w(TAG, "camera: preview request", it) }
    }

    /** The dials on a request: exposure, white balance (if this camera has the mode), and zoom as a centred crop. */
    private fun dials(b: CaptureRequest.Builder) {
        val k = caps ?: return
        val c = controls
        b.set(CaptureRequest.CONTROL_AE_EXPOSURE_COMPENSATION, c.ev.coerceIn(k.ev.lower, k.ev.upper))
        if (c.awb in k.awb) b.set(CaptureRequest.CONTROL_AWB_MODE, c.awb)
        val z = c.zoom.coerceIn(1f, maxOf(1f, k.maxZoom))
        if (z > 1f) {
            val w = (k.active.width() / z).toInt()
            val h = (k.active.height() / z).toInt()
            val x = k.active.left + (k.active.width() - w) / 2
            val y = k.active.top + (k.active.height() - h) / 2
            b.set(CaptureRequest.SCALER_CROP_REGION, Rect(x, y, x + w, y + h))
        }
    }

    /** A viewfinder appeared (`texture`) or went (null): the preview moves there, the camera stays open. */
    fun setPreview(texture: SurfaceTexture?) {
        main.removeCallbacks(closeIdle)
        if (texture == null) main.postDelayed(closeIdle, IDLE_MS)
        cam?.post { if (device != null) startSession(texture) }
    }

    /**
     * Takes a still: an upright JPEG at most [MAX_SIDE] on its long side. Opens the camera first if it
     * isn't (with no preview).
     */
    fun capture(context: Context, onJpeg: (ByteArray) -> Unit, onError: (String) -> Unit) = capture(context, null, onJpeg, onError)

    /** A still through `film` (the camera app's: what its viewfinder shows). */
    fun capture(context: Context, film: Film?, onJpeg: (ByteArray) -> Unit, onError: (String) -> Unit) {
        // One answer, whatever happens: a capture cut short (the camera closed under it) never reports,
        // so a few seconds without the picture is an error.
        var answered = false
        val ok: (ByteArray) -> Unit = { b -> if (!answered) { answered = true; onJpeg(b) } }
        val fail: (String) -> Unit = { m -> if (!answered) { answered = true; onError(m) } }
        main.postDelayed({ fail("The camera didn't answer.") }, CAPTURE_TIMEOUT_MS)
        val shoot = Runnable {
            val d = device
            val s = session
            val r = reader
            if (d == null || s == null || r == null) {
                main.post { fail("The camera isn't open.") }
                return@Runnable
            }
            r.setOnImageAvailableListener({ ir ->
                val image = runCatching { ir.acquireLatestImage() }.getOrNull() ?: return@setOnImageAvailableListener
                val bytes = try {
                    val buf = image.planes[0].buffer
                    ByteArray(buf.remaining()).also { buf.get(it) }
                } finally {
                    image.close()
                }
                ir.setOnImageAvailableListener(null, null)
                val out = runCatching { upright(bytes, film) }.getOrElse { bytes }
                main.post { ok(out) }
            }, cam)
            try {
                val req = d.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE).apply {
                    addTarget(r.surface)
                    previewSurface?.let { addTarget(it) }
                    set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO)
                    set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                    set(CaptureRequest.JPEG_QUALITY, 92.toByte())
                    dials(this)
                }
                s.capture(req.build(), null, cam)
            } catch (e: Exception) {
                Log.w(TAG, "camera: capture", e)
                main.post { fail(e.message ?: "Couldn't take the photo.") }
            }
        }
        if (isOpen) handler().post(shoot) else open(context, null, onReady = { handler().post(shoot) }, onError = fail)
    }

    /**
     * Turned upright (the sensor's mounting), sized down, re-encoded. The same turn facing you: Rabbit's camera
     * already flips the picture when the motor turns it round (an extra 180° made selfies upside down, Max,
     * 2026-10-10).
     */
    private fun upright(jpeg: ByteArray, film: Film?): ByteArray {
        val raw = BitmapFactory.decodeByteArray(jpeg, 0, jpeg.size) ?: return jpeg
        val turn = sensor % 360
        val scale = (MAX_SIDE.toFloat() / maxOf(raw.width, raw.height)).coerceAtMost(1f)
        val m = Matrix().apply {
            postScale(scale, scale)
            postRotate(turn.toFloat())
            // The camera app's selfie, kept as its viewfinder showed it: mirrored (agents' own looks aren't).
            if (film != null && facingUser && mirrorSelfies) postScale(-1f, 1f)
        }
        var out = Bitmap.createBitmap(raw, 0, 0, raw.width, raw.height, m, true)
        raw.recycle()
        if (film != null && (film != Film.STD || controls.gain != 0f)) {
            // Grain the size it looks in the viewfinder: about 480 px wide on the R1's screen.
            val baked = film.bake(out, controls.gain, grainScale = (out.width / 480f).coerceAtLeast(1f))
            if (baked !== out) out.recycle()
            out = baked
        }
        return ByteArrayOutputStream().use { s ->
            out.compress(Bitmap.CompressFormat.JPEG, 85, s)
            out.recycle()
            s.toByteArray()
        }
    }

    /** How a viewfinder of `w` × `h` px shows the preview: turned upright and filling it (centre crop). */
    fun previewTransform(w: Int, h: Int): Matrix {
        val m = Matrix()
        if (w == 0 || h == 0) return m
        // The preview arrives turned upright already in a portrait view, at the preview's aspect turned around.
        val bw = previewSize.height.toFloat()
        val bh = previewSize.width.toFloat()
        val fill = maxOf(w / bw, h / bh)
        m.setScale(fill * bw / w, fill * bh / h, w / 2f, h / 2f)
        // Facing you, the module is upside down. Rabbit's HAL turns its stills round, not the preview (Max, 2026-10-10).
        if (facingUser) m.postRotate(180f, w / 2f, h / 2f)
        // And as in a mirror, as a phone's front camera shows you (Max, 2026-10-10).
        if (facingUser && mirrorSelfies) m.postScale(-1f, 1f, w / 2f, h / 2f)
        return m
    }

    fun close() {
        main.removeCallbacks(closeIdle)
        cam?.post { closeNow() }
    }

    private fun closeNow() {
        runCatching { session?.close() }
        runCatching { device?.close() }
        session = null
        device = null
        opening = false
        previewSurface?.release()
        previewSurface = null
        runCatching { reader?.close() }
        reader = null
        readyWaiters.clear()
    }

    private fun handler(): Handler = cam ?: HandlerThread("r1os-camera").let { t ->
        t.start()
        thread = t
        Handler(t.looper).also { cam = it }
    }
}
