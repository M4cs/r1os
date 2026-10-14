package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Outline
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.ViewOutlineProvider
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.AgentApps
import dev.r1os.host.AgentPointer
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text

/**
 * The app window, small, in the chat of the agent using it (ADR-0061, as the browser's [BrowserPreview]): the app's
 * live picture, with who's using which app under it. A touch opens the full window ([AppWindowScreen]), where you watch
 * and take over. Its frames come only while it's on screen.
 */
@SuppressLint("ViewConstructor")
class AppPreview(context: Context, private val onOpen: () -> Unit) : LinearLayout(context) {
    private val w = context.dpi(BrowserPreview.WIDTH_DP)
    private val dot = View(context)
    private val label = context.text(11f, 700, Theme.FG).keepCase().apply { maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END }
    private val picture = ImageView(context).apply { scaleType = ImageView.ScaleType.FIT_XY }
    private var offFrame: (() -> Unit)? = null
    /** The agent's colour, for its pointer. */
    private var tint = Theme.accent
    /** The agent's pointer over the app (T-137): the app's display pixels, scaled as the picture is. */
    private val pointer = PointerView(context, AgentPointer.app, small = true, tint = { tint }) { x, y, out ->
        val size = AgentApps.size
        if (size.x == 0 || picture.width == 0) false else { out.set(x * picture.width / size.x, y * picture.height / size.y); true }
    }

    init {
        orientation = VERTICAL
        gravity = Gravity.CENTER_HORIZONTAL
        setPadding(context.dpi(4f), context.dpi(4f), context.dpi(4f), context.dpi(6f))
        background = roundRect(Theme.SURFACE, context.dp(16f))
        elevation = context.dp(6f)
        // As tall as the R1's own screen would be at this width, however tall the window is (ADR-0063): a taller app is
        // a narrower picture, not a bigger one.
        val size = AgentApps.size
        val h = w * 4 / 3
        val pw = h * size.x / size.y
        addView(FrameLayout(context).apply {
            clipToOutline = true
            outlineProvider = object : ViewOutlineProvider() {
                override fun getOutline(view: View, outline: Outline) = outline.setRoundRect(0, 0, view.width, view.height, context.dp(12f))
            }
            setBackgroundColor(android.graphics.Color.BLACK)
            addView(picture, FrameLayout.LayoutParams(pw, h))
            addView(pointer, FrameLayout.LayoutParams(pw, h))
        }, LayoutParams(pw, h))
        addView(LinearLayout(context).apply {
            orientation = HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(dot, LayoutParams(context.dpi(7f), context.dpi(7f)).apply { rightMargin = context.dpi(5f) })
            addView(label)
        }, LayoutParams(w, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(5f) })
        setOnClickListener {
            Sounds.play(Sounds.S.OPEN)
            onOpen()
        }
    }

    /** Frames while it's on screen; none once it isn't (battery: architecture §7). */
    override fun onVisibilityAggregated(isVisible: Boolean) {
        super.onVisibilityAggregated(isVisible)
        offFrame?.invoke()
        offFrame = if (isVisible) AgentApps.onFrame { picture.setImageBitmap(it) } else null
    }

    /** Who's using which app. */
    fun update(name: String, color: Int, working: Boolean) {
        label.text = "${name.lowercase()} · ${AgentApps.name?.lowercase() ?: "an app"}"
        tint = color
        dot.background = roundRect(if (working) color else Theme.MUTED, context.dp(4f))
    }
}
