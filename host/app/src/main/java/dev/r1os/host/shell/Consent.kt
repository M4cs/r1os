package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.view.Gravity
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject

/**
 * The consent card (ADR-0023, docs/specs/apps.md §6). The first time an app opens with permissions you
 * haven't decided on, the shell asks over it, in its own UI: the app's page can't draw it or answer it.
 * Each permission is said in r1os's words, big ("get data from wttr.in"), with the app's own reason
 * under it; then **allow** or **don't allow**, for all of them. The answer goes to r1d, and the app's
 * page in Manage apps can change it later.
 */
@SuppressLint("ViewConstructor")
class ConsentCard(
    context: Context,
    private val appId: String,
    name: String,
    icon: String,
    accent: Int,
    private val asks: List<JSONObject>,
    private val onDone: (allowed: Boolean) -> Unit,
) : FrameLayout(context) {
    private val list = FocusList(context).apply { this.accent = accent }
    private var sent = false

    init {
        setBackgroundColor(Theme.BG)
        isClickable = true
        addView(list, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT).apply {
            setMargins(context.dpi(14f), context.dpi(18f), context.dpi(14f), context.dpi(10f))
        })
        list.rebuild {
            val head = LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(context.dpi(6f), 0, 0, context.dpi(10f))
                addView(FrameLayout(context).apply {
                    background = roundRect(Theme.withAlpha(accent, 0.2f), context.dpi(12f).toFloat())
                    addView(ImageView(context).apply { setImageDrawable(Icons.drawable(icon, accent)) },
                        LayoutParams(context.dpi(24f), context.dpi(24f), Gravity.CENTER))
                }, LinearLayout.LayoutParams(context.dpi(44f), context.dpi(44f)))
                addView(context.text(28f, 700).apply {
                    text = name
                    maxLines = 1
                    ellipsize = android.text.TextUtils.TruncateAt.END
                    setPadding(context.dpi(12f), 0, 0, 0)
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            }
            place(head)
            place(context.text(17f, 600, Theme.MUTED).apply {
                text = "would like to"
                setPadding(context.dpi(6f), 0, 0, context.dpi(2f))
            })
            for (p in asks) {
                place(LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL
                    setPadding(context.dpi(6f), context.dpi(6f), context.dpi(6f), context.dpi(8f))
                    addView(context.text(23f, 700).apply { text = p.optString("label"); setLineSpacing(0f, 1.05f) })
                    p.optString("reason").takeIf { it.isNotBlank() }?.let { r ->
                        addView(context.text(16f, 500, Theme.MUTED).apply { text = r; setPadding(0, context.dpi(3f), 0, 0) })
                    }
                })
            }
            button("allow", accent = accent) { decide(true) }
            button("don't allow", primary = false) { decide(false) }
        }
        alpha = 0f
        animate().alpha(1f).setDuration(160).start()
    }

    /** The wheel moves between allow and don't allow; a click decides. Double tap leaves the app (the shell's). */
    fun onEvent(ev: Ev): Boolean {
        when {
            ev.type == "wheel" -> { list.move(ev.int("delta")); return true }
            ev.isTap -> { list.activate(); return true }
        }
        return false
    }

    private fun decide(allow: Boolean) {
        if (sent) return
        sent = true
        val decisions = JSONObject()
        asks.forEach { decisions.put(it.optString("id"), allow) }
        val body = JSONObject().put("decisions", decisions).put("via", "consent")
        R1d.post("/api/apps/${R1d.enc(appId)}/permissions", body, onError = { sent = false }) {
            animate().alpha(0f).setDuration(140).withEndAction { onDone(allow) }.start()
        }
    }
}
