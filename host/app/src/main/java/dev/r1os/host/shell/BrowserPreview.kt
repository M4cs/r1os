package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Outline
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.ViewOutlineProvider
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.AgentBrowser
import dev.r1os.host.AgentPointer
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text

/**
 * The R1's browser, small, in the chat of the agent using it (T-090; Max, 2026-10-11: "show a little preview of the
 * browser and the agent using it instead of replacing the whole screen. Then if we tap into that it will expand and
 * take up the full screen or we can take over control as well there."): the live page (the browser's own view, laid
 * out at full size and scaled down, so nothing is captured or polled), with who's browsing and where under it. A touch
 * opens the full browser ([BrowserScreen]), where you watch and take over by touch.
 */
@SuppressLint("ViewConstructor")
class BrowserPreview(context: Context, private val onOpen: () -> Unit) : LinearLayout(context) {
    private val fullW = context.resources.displayMetrics.widthPixels
    private val fullH = context.resources.displayMetrics.heightPixels - context.dpi(BrowserScreenHeader)
    private val w = context.dpi(WIDTH_DP)
    private val scale = w / fullW.toFloat()
    private val h = (fullH * scale).toInt()
    private val dot = View(context)
    private val label = context.text(11f, 700, Theme.FG).keepCase().apply { maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END }
    /** The page, scaled, with rounded corners. */
    private val page = FrameLayout(context).apply {
        clipChildren = true
        clipToOutline = true
        outlineProvider = object : ViewOutlineProvider() {
            override fun getOutline(view: View, outline: Outline) = outline.setRoundRect(0, 0, view.width, view.height, context.dp(12f))
        }
        setBackgroundColor(android.graphics.Color.WHITE)
    }
    /** The agent's colour, for its pointer. */
    private var tint = Theme.accent
    /** The agent's pointer over the page (T-137): the page's pixels, scaled as the page is. */
    private val pointer = PointerView(context, AgentPointer.browser, small = true, tint = { tint }) { x, y, out -> out.set(x * scale, y * scale); true }
    /** Over the page: a touch opens the full browser rather than clicking the page this small. */
    private val shield = View(context).apply {
        setOnClickListener {
            Sounds.play(Sounds.S.OPEN)
            onOpen()
        }
    }

    init {
        orientation = VERTICAL
        gravity = Gravity.CENTER_HORIZONTAL
        setPadding(context.dpi(4f), context.dpi(4f), context.dpi(4f), context.dpi(6f))
        background = roundRect(Theme.SURFACE, context.dp(16f))
        elevation = context.dp(6f)
        addView(FrameLayout(context).apply {
            addView(page, FrameLayout.LayoutParams(w, h))
            addView(pointer, FrameLayout.LayoutParams(w, h))
            addView(shield, FrameLayout.LayoutParams(w, h))
        }, LayoutParams(w, h))
        addView(LinearLayout(context).apply {
            orientation = HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(dot, LayoutParams(context.dpi(7f), context.dpi(7f)).apply { rightMargin = context.dpi(5f) })
            addView(label)
        }, LayoutParams(w, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(5f) })
        setOnClickListener { shield.performClick() }
    }

    /**
     * The browser's view in here, at full size and scaled down. It doesn't count as watching: the browser still closes
     * after two idle minutes (ADR-0054), and the preview goes with it.
     */
    fun attach() {
        val web = AgentBrowser.view(context)
        if (web.parent === page) return
        (web.parent as? ViewGroup)?.removeView(web)
        web.pivotX = 0f
        web.pivotY = 0f
        web.scaleX = scale
        web.scaleY = scale
        page.addView(web, 0, FrameLayout.LayoutParams(fullW, fullH))
    }

    /** Back behind everything (the chat's leaving the screen), unless the full browser or a close took it already. */
    fun detach() {
        if (page.childCount > 0) AgentBrowser.park()
    }

    /** Who's browsing and where. */
    fun update(name: String, color: Int, working: Boolean) {
        val site = AgentBrowser.url.substringAfter("://").substringBefore("/").removePrefix("www.").ifEmpty { "…" }
        label.text = AgentBrowser.note ?: "${name.lowercase()} · $site"
        tint = color
        dot.background = roundRect(if (working) color else Theme.MUTED, context.dp(4f))
    }

    companion object {
        const val WIDTH_DP = 122f
        /** The full browser's header, which its page sits under: the preview keeps the page's own shape. */
        const val BrowserScreenHeader = 56f
    }
}
