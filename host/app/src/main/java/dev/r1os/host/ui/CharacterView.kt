package dev.r1os.host.ui

import android.animation.Animator
import android.animation.AnimatorListenerAdapter
import android.animation.Keyframe
import android.animation.ObjectAnimator
import android.animation.PropertyValuesHolder
import android.animation.TimeInterpolator
import android.animation.ValueAnimator
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.BlurMaskFilter
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RadialGradient
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.drawable.GradientDrawable
import android.os.SystemClock
import android.util.FloatProperty
import android.util.Property
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.animation.LinearInterpolator
import android.widget.FrameLayout
import android.widget.TextView

/**
 * An agent, drawn natively (ADR-0017; ported from shell/src/ui/Character.tsx and character.css).
 *
 * Nine forms in seven finishes, with eyes and things to wear from the wardrobe ([Wardrobe], T-062: on its head, its
 * face, its cheeks, at its neck, on its back), and one state model: idle, listening, thinking, waiting, speaking,
 * working, sleeping, error. It's built from layers that each draw once (the body, with its finish and
 * texture, is a bitmap): every animation is a property animation (translation, scale, rotation,
 * alpha) of a layer, so the frame never redraws and the R1's A53 stays cool. Only an active
 * character animates (home pauses the ones beside it).
 *
 * All geometry is in the web version's 200-unit box, scaled to [sizePx].
 */
class CharacterView(context: Context, private val sizePx: Int) : FrameLayout(context) {
    private val k = sizePx / 200f

    private var form = "orb"
    private var colorName = "orange"
    private var finish = "soft"
    private var eyeStyle = "classic"
    private var wear: List<String> = emptyList()
    private var state = "idle"
    private var activity: String? = null
    private var unread = 0
    private var c = Theme.colorOf(colorName)
    private var haloColor = c
    private var active = true

    private val halo = Layer { cv, p -> p.style = Paint.Style.STROKE; p.strokeWidth = 2 * k; p.color = haloColor; cv.drawCircle(100 * k, 100 * k, 86 * k, p) }
    private val orbit = Layer { cv, p -> p.color = c; for ((x, y, r) in ORBIT) cv.drawCircle(x * k, y * k, r * k, p) }
    private val float = FrameLayout(context)
    private val body = Layer { cv, _ -> bodyBitmap?.let { cv.drawBitmap(it, 0f, 0f, null) } }
    private val eyes = Layer { cv, _ -> drawEyes(cv) }
    /**
     * What it wears (T-062): wings, a cape or a crown's back behind the body; straps, collars and what's on its cheeks
     * under the eyes; eyewear over them; a hat on top, and the ears again over a hat that asks for it.
     */
    private val behindWear = Layer { cv, _ -> drawWorn(cv, behind = true) }
    private val underWear = Layer { cv, _ -> drawWorn(cv, "back", "neck", "cheeks") }
    private val faceWear = Layer { cv, _ -> drawWorn(cv, "face") }
    private val hatWear = Layer { cv, _ -> drawWorn(cv, "head"); drawEars(cv) }
    private val mouth = Layer { cv, p -> val y = fit().point("mouth").second; p.color = if (form == "bot") c else EYE; cv.drawOval(91 * k, (y - 5) * k, 109 * k, (y + 5) * k, p) }
    private val thought = List(3) { i -> Layer { cv, p -> p.color = c; val (x, y, r) = THOUGHT[i]; cv.drawCircle(x * k, y * k, r * k, p) } }
    private val badge = Layer { cv, p -> drawBadge(cv, p) }
    private val icon = Layer { cv, p -> drawIcon(cv, p) }
    private val unreadView = TextView(context)
    /** What reactions move (ADR-0043): every layer is in it, so a bounce or a spin goes over the state's own motion. */
    private val stage = FrameLayout(context).apply { clipChildren = false; clipToPadding = false }
    private var reaction: Animator? = null
    private val clearFace = Runnable { expression = null }

    private var bodyBitmap: Bitmap? = null
    private val anims = ArrayList<Animator>()

    /**
     * Breathing and glancing around (idle, waiting, asleep) move slowly, so they're drawn [CALM_FRAME_MS] apart instead
     * of every frame: each frame recomposes the whole display, and at 60 a character on screen cost ~22% of all eight
     * cores (T-033). Their animators aren't started: the calm clock ([Calm]) sets their time, on a plain message rather
     * than a frame callback (which wakes the main thread twice: for the message, then for the vsync), and their values
     * move in whole pixels ([snapped]), so most ticks draw nothing. Blinks go on the same clock, a little quicker while
     * one is closing and opening. The active states (listening, thinking, speaking, working) keep 60.
     */
    private val calm = ArrayList<Pair<ValueAnimator, Long>>()
    private var calmStart = 0L
    /** Idle, it blinks now and then: when the next one is, and when the one in progress began (0: none). */
    private var blinkOn = false
    private var nextBlink = 0L
    private var blinkAt = 0L

    /**
     * Its own rhythm (Max, 2026-10-13: "make the different agents on the glance view animate at different intervals
     * instead of them doing the same thing all thet ime"): how fast it breathes and glances around, where in those
     * cycles it starts, and the gaps between its blinks. From its looks, so an agent keeps its rhythm everywhere.
     */
    private var tempo = 1f
    private var lookTempo = 1f
    private var phase = 0L
    private var blinkGaps = kotlin.random.Random(0)

    private fun rhythm(seed: Int) {
        val r = kotlin.random.Random(seed)
        tempo = 0.82f + r.nextFloat() * 0.4f
        lookTempo = 0.8f + r.nextFloat() * 0.55f
        phase = r.nextLong(0, 9000)
        blinkGaps = r
    }

    /** The calm clock's tick: its calm loops' time, and its blink. True while a blink is closing or opening. */
    private fun calmStep(now: Long): Boolean {
        val t = now - calmStart
        for ((a, delay) in calm) {
            // Within one cycle (two when it reverses), so the time stays exact however long it runs.
            val cycle = if (a.repeatMode == ValueAnimator.REVERSE) 2 * a.duration else a.duration
            a.currentPlayTime = (t - delay).coerceAtLeast(0) % cycle.coerceAtLeast(1)
        }
        if (!blinkOn) return false
        if (blinkAt == 0L) {
            if (now < nextBlink) return false
            blinkAt = now
        }
        val p = (now - blinkAt).toFloat() / BLINK_MS
        if (p >= 1f) {
            eyes.scaleY = 1f
            blinkAt = 0L
            nextBlink = now + blinkGaps.nextLong(BLINK_MIN_MS, BLINK_MAX_MS)
            return false
        }
        // Shut by the middle, open again by the end.
        eyes.scaleY = 1f - 0.9f * (if (p < 0.5f) p * 2 else (1 - p) * 2)
        return true
    }

    init {
        clipChildren = false
        clipToPadding = false
        layoutParams = LayoutParams(sizePx, sizePx)
        float.clipChildren = false
        val full = { LayoutParams(sizePx, sizePx) }
        addView(stage, full())
        stage.addView(halo, full())
        stage.addView(orbit, full())
        float.addView(behindWear, full())
        float.addView(body, full())
        float.addView(underWear, full())
        float.addView(eyes, full())
        float.addView(mouth, full())
        float.addView(faceWear, full())
        float.addView(hatWear, full())
        stage.addView(float, full())
        thought.forEach { stage.addView(it, full()) }
        stage.addView(badge, full())
        stage.addView(icon, full())
        unreadView.apply {
            setTextSize(TypedValue.COMPLEX_UNIT_PX, maxOf(context.dp(10f), sizePx * 0.11f))
            typeface = Theme.font(800)
            setTextColor(Color.parseColor("#111111"))
            gravity = Gravity.CENTER
            includeFontPadding = false
            val pad = (sizePx * 0.11f * 0.45f).toInt()
            setPadding(pad, 0, pad, 0)
            visibility = GONE
        }
        val h = (maxOf(context.dp(10f), sizePx * 0.11f) * 1.7f).toInt()
        stage.addView(unreadView, LayoutParams(LayoutParams.WRAP_CONTENT, h).apply { leftMargin = (sizePx * 0.06f).toInt(); topMargin = (sizePx * 0.04f).toInt() })
        unreadView.minWidth = h
        // Pivots: each layer turns and scales about its own centre, like the SVG's fill-box origins.
        halo.pivotX = 100 * k; halo.pivotY = 100 * k
        orbit.pivotX = 100 * k; orbit.pivotY = 100 * k
        float.pivotX = 100 * k; float.pivotY = 100 * k
        thought.forEachIndexed { i, v -> v.pivotX = THOUGHT[i].first * k; v.pivotY = THOUGHT[i].second * k }
        badge.pivotX = BADGE_X * k; badge.pivotY = BADGE_Y * k
        icon.pivotX = BADGE_X * k; icon.pivotY = BADGE_Y * k
        rebuildBody()
        applyState()
    }

    /** Sets what to draw; only what changed is redrawn or re-animated. */
    fun set(
        form: String, color: String, finish: String, state: String, activity: String? = null, unread: Int = 0,
        eyes: String = "classic", wear: List<String> = emptyList(),
    ) {
        val f = if (form in FORMS) form else "orb"
        val fin = if (finish in FINISHES) finish else "soft"
        val e = if (eyes in EYES) eyes else "classic"
        val looks = f != this.form || color != colorName || fin != this.finish || e != eyeStyle || wear != this.wear
        this.eyeStyle = e
        this.wear = wear
        if (looks) rhythm((f + color + fin + e).hashCode())
        val moves = state != this.state || activity != this.activity
        this.form = f
        this.colorName = color
        this.finish = fin
        this.state = state
        this.activity = activity
        c = Theme.colorOf(color)
        if (looks) {
            rebuildBody()
            listOf(halo, orbit, this.eyes, mouth, badge, icon, behindWear, underWear, faceWear, hatWear).forEach { it.invalidate() }
            thought.forEach { it.invalidate() }
        }
        if (looks || moves) applyState()
        setUnread(unread)
    }

    // ---- reactions (ADR-0043) ------------------------------------------------------------------------

    /**
     * A moment's reaction, over its state: a poke's bounce, giggle, wink or dizzy spin, a pet's love, a surprise, a
     * yawn. Transform and opacity only, and only while it's on screen.
     */
    fun react(kind: String) {
        if (!isAttachedToWindow) return
        reaction?.cancel()
        removeCallbacks(clearFace)
        stage.pivotX = 100 * k
        // Turns about its middle (a spin stays on screen when it's peeking up); squashes from its feet.
        stage.pivotY = (if (kind in listOf("spin", "giggle", "wink")) 100f else 168f) * k
        val face: String?
        val ms: Long
        val holders: Array<PropertyValuesHolder> = when (kind) {
            "bounce" -> {
                face = null; ms = 650
                arrayOf(
                    kf(View.TRANSLATION_Y, 0f to 0f, 0.35f to -20 * k, 0.7f to 0f, 0.85f to -5 * k, 1f to 0f),
                    kf(View.SCALE_Y, 0f to 1f, 0.12f to 0.9f, 0.35f to 1.08f, 0.7f to 0.88f, 0.86f to 1.03f, 1f to 1f),
                    kf(View.SCALE_X, 0f to 1f, 0.12f to 1.08f, 0.35f to 0.94f, 0.7f to 1.1f, 0.86f to 0.98f, 1f to 1f),
                )
            }
            "giggle" -> {
                face = "happy"; ms = 820
                arrayOf(
                    kf(View.ROTATION, 0f to 0f, 0.15f to -7f, 0.3f to 7f, 0.45f to -6f, 0.6f to 6f, 0.8f to -3f, 1f to 0f),
                    kf(View.TRANSLATION_Y, 0f to 0f, 0.2f to -4 * k, 0.4f to 0f, 0.6f to -4 * k, 0.8f to 0f, 1f to 0f),
                )
            }
            "wink" -> { face = "wink"; ms = 700; arrayOf(kf(View.ROTATION, 0f to 0f, 0.4f to 7f, 1f to 0f)) }
            "spin" -> { face = "dizzy"; ms = 1300; arrayOf(kf(View.ROTATION, 0f to 0f, 0.7f to 360f, 0.8f to 352f, 0.9f to 363f, 1f to 360f)) }
            "love" -> { face = "love"; ms = 1000; arrayOf(scale(0f to 1f, 0.25f to 1.08f, 0.5f to 1f, 0.75f to 1.08f, 1f to 1f)) }
            "surprised" -> {
                face = "surprised"; ms = 450
                arrayOf(kf(View.TRANSLATION_Y, 0f to 0f, 0.3f to -12 * k, 1f to 0f), scale(0f to 1f, 0.3f to 1.06f, 1f to 1f))
            }
            "yawn" -> {
                face = "sleepy"; ms = 1600
                mouth.animate().cancel()
                mouth.animate().alpha(1f).scaleY(1.7f).setDuration(500).withEndAction {
                    mouth.animate().alpha(if (state == "speaking") 1f else 0f).scaleY(if (state == "speaking") 1f else 0.4f).setStartDelay(500).setDuration(400).start()
                }.start()
                arrayOf(kf(View.SCALE_Y, 0f to 1f, 0.4f to 1.07f, 0.7f to 1.07f, 1f to 1f), kf(View.ROTATION, 0f to 0f, 0.4f to -3f, 1f to 0f))
            }
            else -> return
        }
        expression = face
        val a = ObjectAnimator.ofPropertyValuesHolder(stage, *holders).apply { duration = ms }
        a.addListener(object : AnimatorListenerAdapter() {
            override fun onAnimationEnd(animation: Animator) = rest()
            override fun onAnimationCancel(animation: Animator) = rest()
        })
        reaction = a
        a.start()
        postDelayed(clearFace, maxOf(ms, 900L))
        if (kind == "love") hearts()
    }

    private fun rest() {
        stage.translationX = 0f; stage.translationY = 0f; stage.rotation = 0f; stage.scaleX = 1f; stage.scaleY = 1f
    }

    /** Its eyes glance up or down for a moment (at the widget the wheel turned to), in the 200-unit box. */
    fun glanceAt(dy: Float, hold: Long = 900) {
        if (!isAttachedToWindow) return
        eyes.animate().cancel()
        eyes.animate().translationY(dy * k).setStartDelay(0).setDuration(180).setInterpolator(Theme.EASE_OUT).withEndAction {
            eyes.animate().translationY(0f).setStartDelay(hold).setDuration(260).start()
        }.start()
    }

    /** A pet's hearts, floating up from its head. */
    private fun hearts() {
        val pink = Color.parseColor("#ff4d6d")
        repeat(3) { i ->
            val size = (18 * k * (1f + 0.2f * (i % 2))).toInt()
            val h = object : View(context) {
                private val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = pink }
                override fun onDraw(c: Canvas) = c.drawPath(heart(width / 2f, height / 2f, width * 0.42f), p)
            }
            addView(h, LayoutParams(size, size).apply {
                leftMargin = ((100 + (i - 1) * 30) * k - size / 2).toInt()
                topMargin = (44 * k).toInt()
            })
            h.alpha = 0f
            h.animate().alpha(1f).translationY(-44 * k).setStartDelay(i * 160L).setDuration(900).setInterpolator(Theme.EASE_OUT).withEndAction {
                h.animate().alpha(0f).setStartDelay(0).setDuration(250).withEndAction { removeView(h) }.start()
            }.start()
        }
    }

    private var pokes = 0
    private var lastPoke = 0L

    /** A tap: taps in a row build up, from a bounce to a giggle, a wink, and then it spins. */
    fun poke() {
        val now = android.os.SystemClock.uptimeMillis()
        pokes = if (now - lastPoke < POKE_RUN_MS) pokes + 1 else 1
        lastPoke = now
        Sounds.play(Sounds.S.POP)
        react(
            when {
                pokes >= 4 -> { pokes = 0; "spin" }
                pokes == 3 -> "wink"
                pokes == 2 -> "giggle"
                else -> "bounce"
            },
        )
    }

    /**
     * Touch on it (ADR-0043): a tap pokes it (or does [onTap]: the glance screen opens its chat, T-122), a stroke across
     * it pets it, a long press does [onLong]. Only while [enabled] says so (home's neighbours are selected by a tap instead).
     */
    fun pokeable(enabled: () -> Boolean = { true }, onLong: (() -> Unit)? = null, onTap: (() -> Unit)? = null) {
        var downX = 0f
        var downY = 0f
        var petted = false
        var longFired = false
        val slop = context.dp(10f)
        val longPress = Runnable {
            if (onLong != null) {
                longFired = true
                onLong()
            }
        }
        setOnTouchListener { _, e ->
            if (!enabled()) return@setOnTouchListener false
            when (e.actionMasked) {
                android.view.MotionEvent.ACTION_DOWN -> {
                    downX = e.x; downY = e.y; petted = false; longFired = false
                    postDelayed(longPress, LONG_MS)
                }
                android.view.MotionEvent.ACTION_MOVE -> {
                    val dx = Math.abs(e.x - downX)
                    if (dx > slop || Math.abs(e.y - downY) > slop) removeCallbacks(longPress)
                    if (!petted && dx > context.dp(36f)) {
                        petted = true
                        react("love")
                    }
                }
                android.view.MotionEvent.ACTION_UP -> {
                    removeCallbacks(longPress)
                    if (!petted && !longFired && Math.abs(e.x - downX) < slop && Math.abs(e.y - downY) < slop) {
                        if (onTap != null) onTap() else poke()
                    }
                }
                android.view.MotionEvent.ACTION_CANCEL -> removeCallbacks(longPress)
            }
            true
        }
    }

    /** Its eyes open (1) to shut (0), for frames drawn offscreen (the boot animation, T-029). */
    fun blinkPose(open: Float) {
        eyes.scaleY = open
    }

    /** Only the character on screen animates; the others hold a still pose. */
    fun setActive(on: Boolean) {
        if (on == active) return
        active = on
        applyState()
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        Dim.unwatch(onDim)
        stopAnims()
    }

    /** The display dimmed, or input brought it back (T-036): a still pose, or its life again. */
    private val onDim: () -> Unit = { applyState() }

    /**
     * On screen: in a visible window, itself and its parents shown. The loops run only then. With the display off the
     * activity stops but its views stay attached, and characters nobody had turned off kept looping on every frame: the
     * host's main thread at ~15% of a core while the R1 slept (T-033, 2026-10-11).
     */
    private var seen = false

    override fun onVisibilityAggregated(isVisible: Boolean) {
        super.onVisibilityAggregated(isVisible)
        if (isVisible == seen) return
        seen = isVisible
        if (isVisible) applyState() else stopAnims()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        Dim.watch(onDim)
        if (anims.isEmpty()) applyState()
    }

    private fun setUnread(n: Int) {
        if (n == unread && (n > 0) == (unreadView.visibility == VISIBLE)) return
        val appearing = n > 0 && unread == 0
        unread = n
        if (n <= 0) {
            unreadView.visibility = GONE
            return
        }
        unreadView.text = if (n > 9) "9+" else n.toString()
        unreadView.background = GradientDrawable().apply {
            cornerRadius = 999f
            setColor(c)
            setStroke(context.dpi(3f), Theme.BG)
        }
        unreadView.visibility = VISIBLE
        if (appearing) {
            unreadView.scaleX = 0f; unreadView.scaleY = 0f
            unreadView.animate().scaleX(1f).scaleY(1f).setDuration(300).setInterpolator(Theme.POP).start()
        }
    }

    // ---- drawing --------------------------------------------------------------------------------

    private fun fit(): Wardrobe.Form = Wardrobe.catalog.form(form)

    /** The colours and eye spots it's drawn with. Eyewear sits on its own eyes' spots, whatever face a reaction shows. */
    private fun look() = Wardrobe.Look(form, fit(), c, Wardrobe.catalog.eyes(eyeStyle).spots)

    /** Its eyes, in its style ([expression] can stand in for a moment: happy, wink, dizzy, surprised, love). */
    private fun drawEyes(cv: Canvas) {
        val style = Wardrobe.catalog.eyes(expression ?: eyeStyle)
        cv.save()
        cv.scale(k, k)
        Wardrobe.drawAt(cv, style.layers, Wardrobe.Look(form, fit(), c, style.spots), "face")
        cv.restore()
    }

    /** What it wears in these slots, at their pins (or, `behind`, every item's part behind the body). */
    private fun drawWorn(cv: Canvas, vararg slots: String, behind: Boolean = false) {
        val worn = Wardrobe.catalog.worn(wear)
        if (worn.isEmpty()) return
        val look = look()
        val pin = { slot: String -> Wardrobe.catalog.slot(slot)?.pin ?: "face" }
        cv.save()
        cv.scale(k, k)
        if (behind) {
            for (i in worn.asReversed()) Wardrobe.drawAt(cv, i.behind, look, pin(i.slot))
        } else {
            for (s in slots) for (i in worn) if (i.slot == s) Wardrobe.drawAt(cv, i.layers, look, pin(s))
        }
        cv.restore()
    }

    /** Its ears again, over a hat that asks for it: the body inside its ears and outside its head (a bunny's, through a beanie). */
    private fun drawEars(cv: Canvas) {
        val f = fit()
        val ears = f.ears ?: return
        if (Wardrobe.catalog.worn(wear).firstOrNull { it.slot == "head" }?.ears != true) return
        val bmp = bodyBitmap ?: return
        val km = Matrix().apply { setScale(k, k) }
        cv.save()
        cv.clipPath(Path(f.body).apply { transform(km) })
        cv.clipPath(Path(ears).apply { transform(km) })
        cv.clipOutPath(Path(f.head).apply { transform(km) })
        cv.drawBitmap(bmp, 0f, 0f, null)
        cv.restore()
    }

    /** A heart about (x, y), `r` across half its width. */
    private fun heart(x: Float, y: Float, r: Float) = Path().apply {
        moveTo(x, y + r * 0.9f)
        cubicTo(x - r * 1.4f, y - r * 0.1f, x - r * 0.7f, y - r * 1.2f, x, y - r * 0.45f)
        cubicTo(x + r * 0.7f, y - r * 1.2f, x + r * 1.4f, y - r * 0.1f, x, y + r * 0.9f)
        close()
    }

    private fun drawBadge(cv: Canvas, p: Paint) {
        p.style = Paint.Style.FILL
        p.color = Color.parseColor("#0e1013")
        cv.drawCircle(BADGE_X * k, BADGE_Y * k, 17 * k, p)
        p.style = Paint.Style.STROKE
        p.strokeWidth = 3 * k
        p.color = c
        cv.drawCircle(BADGE_X * k, BADGE_Y * k, 17 * k, p)
    }

    private fun drawIcon(cv: Canvas, p: Paint) {
        val kind = badgeKind() ?: return
        cv.save()
        cv.translate(BADGE_X * k, BADGE_Y * k)
        cv.scale(k, k)
        p.color = c
        p.strokeWidth = 2.4f
        p.strokeCap = Paint.Cap.ROUND
        p.strokeJoin = Paint.Join.ROUND
        val (stroked, filled) = ICONS.getValue(kind)
        p.style = Paint.Style.STROKE
        stroked.forEach { cv.drawPath(it, p) }
        p.style = Paint.Style.FILL
        filled.forEach { cv.drawPath(it, p) }
        cv.restore()
    }

    /** The body, its finish and texture (and the bot's antenna and screen), drawn once. */
    private fun rebuildBody() = traced("r1os character body") {
        val bmp = Bitmap.createBitmap(sizePx, sizePx, Bitmap.Config.ARGB_8888)
        val cv = Canvas(bmp)
        val shape = Path(fit().body).apply { transform(Matrix().apply { setScale(k, k) }) }
        val bounds = RectF().also { shape.computeBounds(it, true) }
        // drop-shadow(0 6px 18px c@45%), sized for the 176-dp home character.
        val scale = sizePx / context.dp(176f)
        val blur = 18f * context.resources.displayMetrics.density * scale
        if (blur >= 1f) {
            val shadow = Paint(Paint.ANTI_ALIAS_FLAG).apply {
                color = Theme.withAlpha(c, 0.45f)
                maskFilter = BlurMaskFilter(blur, BlurMaskFilter.Blur.NORMAL)
            }
            cv.save()
            cv.translate(0f, 6f * context.resources.displayMetrics.density * scale)
            cv.drawPath(shape, shadow)
            cv.restore()
        }
        val p = Paint(Paint.ANTI_ALIAS_FLAG)
        if (form == "bot") {
            p.color = c
            p.style = Paint.Style.STROKE
            p.strokeWidth = 4 * k
            p.strokeCap = Paint.Cap.ROUND
            cv.drawLine(100 * k, 34 * k, 100 * k, 52 * k, p)
            p.style = Paint.Style.FILL
            cv.drawCircle(100 * k, 30 * k, 7 * k, p)
        }
        p.style = Paint.Style.FILL
        p.shader = fillShader(bounds)
        cv.drawPath(shape, p)
        // Fine grain, as soft light, at the screen's pixels whatever the size (ADR-0043's update).
        texture()?.let { (kind, amount) -> cv.drawPath(shape, Noise.paint(kind, amount).apply { isAntiAlias = true }) }
        if (form == "bot") {
            p.shader = null
            p.color = Color.argb(140, 0, 0, 0)
            cv.drawRoundRect(58 * k, 70 * k, 142 * k, 126 * k, 18 * k, 18 * k, p)
        }
        bodyBitmap?.recycle()
        bodyBitmap = bmp
        body.invalidate()
    }

    /** The finish's gradient, in the shape's bounding box like SVG's objectBoundingBox units. */
    private fun fillShader(b: RectF): Shader {
        val white = Color.WHITE
        val black = Color.BLACK
        fun radial(cx: Float, cy: Float, r: Float, colors: IntArray, stops: FloatArray?) =
            RadialGradient(cx, cy, r, colors, stops, Shader.TileMode.CLAMP).apply {
                setLocalMatrix(Matrix().apply { setScale(b.width(), b.height()); postTranslate(b.left, b.top) })
            }
        fun linear(colors: IntArray) = LinearGradient(b.left, b.top, b.right, b.bottom, colors, null, Shader.TileMode.CLAMP)
        return when (finish) {
            "matte" -> radial(0.40f, 0.35f, 0.80f, intArrayOf(Theme.mix(c, white, 0.12f), Theme.mix(c, black, 0.12f)), null)
            "gradient" -> linear(intArrayOf(Theme.mix(c, white, 0.15f), c, Theme.hue(c, -42f)))
            "grainy" -> linear(intArrayOf(Theme.mix(c, white, 0.10f), c, Theme.hue(c, -48f)))
            "aurora" -> linear(intArrayOf(Theme.hue(Theme.mix(c, white, 0.2f), 55f), c, Theme.hue(c, -70f)))
            else -> radial(0.38f, 0.32f, 0.78f, intArrayOf(Theme.mix(c, white, 0.55f), c, Theme.mix(c, black, 0.28f)), floatArrayOf(0f, 0.32f, 1f))
        }
    }

    /** The finish's grain, and how much of it. */
    private fun texture(): Pair<Noise.Kind, Float>? = when (finish) {
        "grain" -> Noise.Kind.GRAIN to 0.9f
        "grainy" -> Noise.Kind.GRAIN to 0.8f
        "noise" -> Noise.Kind.NOISE to 1f
        else -> null
    }

    private fun badgeKind(): String? = when (state) {
        "working" -> activity ?: "apps"
        "waiting" -> "waiting"
        else -> null
    }

    // ---- animation --------------------------------------------------------------------------------

    private fun stopAnims() {
        anims.forEach { it.cancel() }
        anims.clear()
        calm.clear()
        blinkOn = false
        blinkAt = 0L
        Calm.remove(this)
    }

    // ---- calm life, at a low frame rate (T-094) ----------------------------------------------------

    private fun calmLoop(a: ValueAnimator, delay: Long) {
        // Each starts somewhere of its own in its cycles.
        if (calm.isEmpty()) calmStart = SystemClock.uptimeMillis() - phase
        calm += a to delay
        Calm.add(this)
    }

    /** A quick blink now and then, its own gaps between them. */
    private fun blinks() {
        blinkOn = true
        blinkAt = 0L
        nextBlink = SystemClock.uptimeMillis() + blinkGaps.nextLong(BLINK_FIRST_MS, BLINK_MAX_MS)
        Calm.add(this)
    }

    /**
     * One clock for every character's calm life (T-157): each keeps its own rhythm, but they all move in the same
     * message, so characters out of step still draw in one frame a tick, not a frame each (four docked on the glance
     * screen drew ~19 frames a second on four clocks). Quicker while any of them is blinking; stopped with none to move.
     */
    private object Calm {
        private val views = LinkedHashSet<CharacterView>()
        private val main = android.os.Handler(android.os.Looper.getMainLooper())
        private var running = false
        private val tick = object : Runnable {
            override fun run() {
                if (views.isEmpty()) {
                    running = false
                    return
                }
                val now = SystemClock.uptimeMillis()
                var quick = false
                for (v in views.toList()) if (v.calmStep(now)) quick = true
                main.postDelayed(this, if (quick) BLINK_FRAME_MS else CALM_FRAME_MS)
            }
        }

        fun add(v: CharacterView) {
            views += v
            if (!running) {
                running = true
                main.post(tick)
            }
        }

        fun remove(v: CharacterView) {
            views -= v
            if (views.isEmpty() && running) {
                running = false
                main.removeCallbacks(tick)
            }
        }
    }

    /** Puts every layer back to rest, then starts the state's animations (if active). */
    private fun applyState() {
        stopAnims()
        for (v in listOf<View>(float, eyes, mouth, halo, orbit, badge, icon) + thought) {
            v.animate().cancel()
            v.translationX = 0f; v.translationY = 0f; v.scaleX = 1f; v.scaleY = 1f; v.rotation = 0f; v.alpha = 1f
        }
        mouth.alpha = 0f; mouth.scaleY = 0.4f
        mouth.pivotX = 100 * k; mouth.pivotY = fit().point("mouth").second * k
        eyes.pivotX = 100 * k; eyes.pivotY = fit().point("face").second * k
        halo.alpha = 0f
        orbit.alpha = 0f
        thought.forEach { it.alpha = 0f }
        haloColor = if (state == "error") ERROR else c
        halo.invalidate()
        val kind = badgeKind()
        badge.visibility = if (kind != null) VISIBLE else GONE
        icon.visibility = badge.visibility
        icon.invalidate()

        // Still poses that hold even when inactive.
        when (state) {
            "listening" -> { float.scaleX = 1.06f; float.scaleY = 1.06f }
            "sleeping" -> { eyes.scaleY = 0.12f; float.alpha = 0.55f }
            "waiting" -> { eyes.scaleY = 0.5f; float.alpha = 0.85f }
            "error" -> halo.alpha = 0.8f
            "speaking" -> mouth.alpha = 1f
            "working" -> orbit.alpha = 0.9f
        }
        // Dimmed (T-036): nobody's looking, so it holds still until the next input.
        if (!active || !isAttachedToWindow || !seen || Dim.dimmed) return

        when (state) {
            "idle" -> { breathe(3600, calm = true); if (blinking()) blinks(); lookAround() }
            "listening" -> {
                loop(halo, 1100, interp = Theme.EASE_OUT,
                    holder = arrayOf(kf(View.ALPHA, 0f to 0.8f, 1f to 0f, ease = null), scale(0f to 0.8f, 1f to 1.18f, ease = null)))
                if (blinking()) blink()
            }
            "thinking" -> {
                loop(eyes, 2400, kf(View.TRANSLATION_X, 0f to 0f, 0.3f to -8 * k, 0.65f to 8 * k, 1f to 0f),
                    kf(View.TRANSLATION_Y, 0f to 0f, 0.3f to -6 * k, 0.65f to -4 * k, 1f to 0f))
                loop(float, 2400, kf(View.ROTATION, 0f to 0f, 0.5f to -4f, 1f to 0f), kf(View.TRANSLATION_Y, 0f to 0f, 0.5f to -3 * k, 1f to 0f))
                thought.forEachIndexed { i, v ->
                    loop(v, 1500, kf(View.ALPHA, 0f to 0f, 0.35f to 0.9f, 0.7f to 0.9f, 1f to 0f),
                        kf(View.TRANSLATION_Y, 0f to 4 * k, 0.35f to 0f, 0.7f to 0f, 1f to 4 * k),
                        scale(0f to 0.6f, 0.35f to 1f, 0.7f to 1f, 1f to 0.6f), delay = i * 200L)
                }
            }
            "speaking" -> {
                loop(mouth, 320, kf(View.SCALE_Y, 0f to 0.35f, 1f to 1.2f), reverse = true)
                loop(float, 640, kf(View.TRANSLATION_Y, 0f to 0f, 0.5f to -4 * k, 1f to 0f))
            }
            "working" -> {
                loop(orbit, 1600, kf(View.ROTATION, 0f to 0f, 1f to 360f, ease = null))
                when (activity) {
                    "build" -> loop(float, 700, kf(View.TRANSLATION_Y, 0f to 0f, 0.5f to 3 * k, 1f to 0f), kf(View.ROTATION, 0f to 0f, 0.5f to 2f, 1f to 0f))
                    else -> breathe(2000)
                }
                when (activity) {
                    "search" -> loop(eyes, 1600, kf(View.TRANSLATION_X, 0f to -9 * k, 0.5f to 9 * k, 1f to -9 * k))
                    "files" -> loop(eyes, 2000, kf(View.TRANSLATION_X, 0f to -6 * k, 0.33f to 0f, 0.66f to 6 * k, 1f to 6 * k, ease = STEP),
                        kf(View.TRANSLATION_Y, 0f to -3 * k, 0.33f to 0f, 0.66f to 3 * k, 1f to 3 * k, ease = STEP))
                    "shell" -> loop(eyes, 1000, kf(View.TRANSLATION_Y, 0f to 0f, 0.5f to 5 * k, 1f to 0f), kf(View.SCALE_Y, 0f to 1f, 0.5f to 0.7f, 1f to 1f))
                    else -> if (blinking()) blink()
                }
                if (activity == "memory") {
                    loop(halo, 1800, kf(View.ALPHA, 0f to 0.15f, 0.5f to 0.7f, 1f to 0.15f), scale(0f to 0.94f, 0.5f to 1.02f, 1f to 0.94f))
                }
                badgeIn()
                iconMotion()
            }
            "sleeping" -> breathe(6000, calm = true)
            "waiting" -> {
                breathe(4800, calm = true)
                badgeIn()
                iconMotion()
            }
            "error" -> {
                val a = ObjectAnimator.ofPropertyValuesHolder(float, kf(View.TRANSLATION_X, 0f to 0f, 0.25f to -6 * k, 0.75f to 6 * k, 1f to 0f))
                a.duration = 500
                a.repeatCount = 1
                anims += a
                a.start()
            }
        }
    }

    /** Eyes already half shut (happy, sleepy: the wardrobe's `blink: false`) don't blink; they still look around. */
    private fun blinking() = Wardrobe.catalog.eyes(eyeStyle).blink

    /** Idle life (ADR-0043): now and then its eyes glance one way, then the other. */
    private fun lookAround() = loop(
        eyes, (9000 * lookTempo).toLong(),
        kf(View.TRANSLATION_X, 0f to 0f, 0.42f to 0f, 0.46f to -4 * k, 0.6f to -4 * k, 0.64f to 0f, 0.8f to 0f, 0.84f to 4 * k, 0.94f to 4 * k, 0.97f to 0f, 1f to 0f),
        calm = true,
    )

    private fun breathe(ms: Long, calm: Boolean = false) =
        loop(float, ms, kf(View.TRANSLATION_Y, 0f to 0f, 0.5f to -5 * k, 1f to 0f), scale(0f to 1f, 0.5f to 1.02f, 1f to 1f), calm = calm)

    private fun blink() = loop(eyes, 5200, kf(View.SCALE_Y, 0f to 1f, 0.46f to 1f, 0.48f to 0.1f, 0.5f to 1f, 1f to 1f))

    private fun badgeIn() {
        badge.scaleX = 0f; badge.scaleY = 0f
        icon.scaleX = 0f; icon.scaleY = 0f
        for (v in listOf(badge, icon)) {
            val pop = ObjectAnimator.ofPropertyValuesHolder(v, scale(0f to 0f, 1f to 1f, ease = null))
            pop.duration = 260
            pop.interpolator = Theme.POP
            anims += pop
            pop.start()
        }
        loop(badge, 1600, kf(View.TRANSLATION_Y, 0f to 0f, 0.5f to -3 * k, 1f to 0f), delay = 260)
    }

    private fun iconMotion() {
        when (badgeKind()) {
            "search" -> loop(icon, 1600, kf(View.TRANSLATION_X, 0f to -2 * k, 0.5f to 2 * k, 1f to -2 * k), kf(View.ROTATION, 0f to -12f, 0.5f to 12f, 1f to -12f), delay = 260)
            "build" -> loop(icon, 700, kf(View.ROTATION, 0f to 0f, 0.5f to -28f, 1f to 0f), delay = 260)
            "memory" -> loop(icon, 1800, scale(0f to 1f, 0.5f to 1.3f, 1f to 1f), kf(View.ROTATION, 0f to 0f, 0.5f to 45f, 1f to 0f), delay = 260)
            "waiting" -> loop(icon, 2400, kf(View.ROTATION, 0f to 0f, 0.4f to 0f, 0.6f to 180f, 1f to 180f), delay = 260)
            else -> Unit
        }
    }

    private fun loop(
        v: View,
        ms: Long,
        vararg holder: PropertyValuesHolder,
        delay: Long = 0,
        reverse: Boolean = false,
        interp: TimeInterpolator = LinearInterpolator(),
        calm: Boolean = false,
    ) {
        if (calm) holder.forEach(::snapped)
        val a = ObjectAnimator.ofPropertyValuesHolder(v, *holder)
        a.duration = ms
        a.repeatCount = ValueAnimator.INFINITE
        a.repeatMode = if (reverse) ValueAnimator.REVERSE else ValueAnimator.RESTART
        a.interpolator = interp
        // Calm (T-094): drawn at a low frame rate by the calm clock, not started; at its own tempo.
        if (calm) {
            a.duration = (ms * tempo).toLong()
            calmLoop(a, delay)
            return
        }
        a.startDelay = delay
        anims += a
        a.start()
    }

    /**
     * A calm loop's values on a whole-pixel grid (T-094; a pixel on the R1 is ~0.09 mm). A view set to the value it
     * already has isn't redrawn, so a frame is drawn only when something has visibly moved: none while the eyes hold a
     * glance, a few a second through a breath. A FloatProperty, or the holder keeps setting through the one it was made
     * with.
     */
    private fun snapped(h: PropertyValuesHolder) {
        val (prop, step) = when (h.propertyName) {
            "translationX" -> View.TRANSLATION_X to 1f
            "translationY" -> View.TRANSLATION_Y to 1f
            // Scaled from the middle: a pixel at each edge.
            "scaleXY" -> SCALE_XY to 2f / sizePx
            "scaleY" -> View.SCALE_Y to 2f / sizePx
            else -> return
        }
        h.setProperty(object : FloatProperty<View>(h.propertyName) {
            override fun get(v: View): Float = prop.get(v)
            override fun setValue(v: View, value: Float) = prop.set(v, Math.round(value / step) * step)
        })
    }

    private fun loop(v: View, ms: Long, interp: TimeInterpolator, holder: Array<PropertyValuesHolder>) = loop(v, ms, *holder, interp = interp)

    /** CSS-style keyframes: each segment eased like `ease-in-out` unless told otherwise. */
    private fun kf(prop: Property<View, Float>, vararg pts: Pair<Float, Float>, ease: TimeInterpolator? = Theme.EASE_IN_OUT): PropertyValuesHolder {
        val frames = pts.map { (t, v) -> Keyframe.ofFloat(t, v).also { if (ease != null) it.interpolator = ease } }
        return PropertyValuesHolder.ofKeyframe(prop, *frames.toTypedArray())
    }

    private fun scale(vararg pts: Pair<Float, Float>, ease: TimeInterpolator? = Theme.EASE_IN_OUT): PropertyValuesHolder {
        // X and Y together: one holder per axis would need two animators.
        val frames = pts.map { (t, v) -> Keyframe.ofFloat(t, v).also { if (ease != null) it.interpolator = ease } }
        return PropertyValuesHolder.ofKeyframe(SCALE_XY, *frames.toTypedArray())
    }

    /** A full-size layer that draws once. */
    private inner class Layer(private val paint: (Canvas, Paint) -> Unit) : View(context) {
        private val p = Paint(Paint.ANTI_ALIAS_FLAG)
        override fun onDraw(canvas: Canvas) {
            p.reset()
            p.isAntiAlias = true
            paint(canvas, p)
        }
    }

    /** A moment's face over its eyes (Reactions, ADR-0043): happy, wink, dizzy, surprised, love; null: its own. */
    var expression: String? = null
        set(v) {
            if (v == field) return
            field = v
            eyes.invalidate()
        }

    companion object {
        /** Calm life's ticks (T-094): 15 a second, and a frame only on the ones that move something by a pixel. */
        const val CALM_FRAME_MS = 66L
        /** The calm clock's ticks while a blink closes and opens (30 a second). */
        const val BLINK_FRAME_MS = 33L
        const val BLINK_MS = 210L
        /** The gaps between one character's blinks (each its own), and before its first. */
        const val BLINK_MIN_MS = 3500L
        const val BLINK_MAX_MS = 8000L
        const val BLINK_FIRST_MS = 800L
        /** The forms and the eyes you can pick (the wardrobe's, T-062). */
        val FORMS: List<String> get() = Wardrobe.catalog.formIds
        val FINISHES = listOf("soft", "matte", "grain", "noise", "gradient", "grainy", "aurora")
        val EYES: List<String> get() = Wardrobe.catalog.eyeIds
        private val EYE = Color.parseColor("#141414")
        private val ERROR = Color.parseColor("#f2555a")
        /** Taps closer together than this are a run: each one reacts a little more. */
        private const val POKE_RUN_MS = 1300L
        private const val LONG_MS = 550L
        private const val BADGE_X = 156f
        private const val BADGE_Y = 48f
        private val ORBIT = listOf(Triple(100f, 12f, 5f), Triple(188f, 100f, 4f), Triple(100f, 188f, 3f))
        private val THOUGHT = listOf(Triple(146f, 50f, 4f), Triple(158f, 36f, 5.5f), Triple(172f, 20f, 7f))
        /** Holds a value until the end of its segment (CSS `steps(1)`). */
        private val STEP = TimeInterpolator { if (it < 1f) 0f else 1f }

        private val SCALE_XY = object : Property<View, Float>(Float::class.java, "scaleXY") {
            override fun get(v: View) = v.scaleX
            override fun set(v: View, value: Float) { v.scaleX = value; v.scaleY = value }
        }

        /** The badge's line icons, around (0, 0): stroked paths, then filled ones (Character.tsx ICONS). */
        private val ICONS: Map<String, Pair<List<Path>, List<Path>>> = mapOf(
            "search" to (listOf(Path().apply { addCircle(-2f, -2f, 5.5f, Path.Direction.CW) }, SvgPath.parse("M2.2 2.2 L7 7")) to emptyList()),
            "build" to (listOf(SvgPath.parse("M-6.5 6.5 L1.5 -1.5")) to listOf(SvgPath.parse("M-1 -6 L4 -7.5 L7.5 -4 L6 1 Z"))),
            "memory" to (emptyList<Path>() to listOf(SvgPath.parse("M0 -8C1 -2 2 -1 8 0C2 1 1 2 0 8C-1 2 -2 1 -8 0C-2 -1 -1 -2 0 -8Z"))),
            "files" to (listOf(SvgPath.parse("M-5 -7.5 H2 L5.5 -4 V7.5 H-5 Z"), SvgPath.parse("M-2 0 H2.5 M-2 3.5 H2.5")) to emptyList()),
            "shell" to (listOf(SvgPath.parse("M-6 -4 L-1.5 0 L-6 4 M0.5 5 H6.5")) to emptyList()),
            "apps" to (listOf(
                Path().apply {
                    for ((x, y) in listOf(-6.5f to -6.5f, 1.5f to -6.5f, -6.5f to 1.5f, 1.5f to 1.5f)) addRoundRect(x, y, x + 5f, y + 5f, 1.2f, 1.2f, Path.Direction.CW)
                },
            ) to emptyList()),
            "waiting" to (listOf(SvgPath.parse("M-5 -7 H5 M-5 7 H5 M-4 -7 C-4 -2 4 2 4 7 M4 -7 C4 -2 -4 2 -4 7")) to emptyList()),
        )

        /** The activity a tool stands for (chat `tool` events carry the tool's name). */
        fun activityOf(tool: String?): String? = when {
            tool.isNullOrEmpty() -> null
            tool == "WebSearch" || tool == "WebFetch" || tool == "web_search" || tool == "read_feed" -> "search"
            tool == "create_app" || tool == "edit_app" || tool == "undo_app_change" -> "build"
            tool == "remember" || tool == "list_memories" || tool == "forget" -> "memory"
            tool == "Bash" -> "shell"
            tool in listOf("Read", "Write", "Edit", "Glob", "Grep") -> "files"
            else -> "apps"
        }
    }
}
