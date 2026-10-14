package dev.r1os.host.shell

import android.text.TextUtils
import android.view.Gravity
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.Notifications
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text

/**
 * What's waiting: the reminders you haven't seen (an agent's or an app's, T-123: banners go by themselves, these stay
 * until seen), then Android apps' notifications ([Notifications]), newest first. The wheel moves between them; a click
 * opens one (an Android app comes up where it was; an app's reminder opens the app) and a reminder is seen. From the
 * launcher, or the glance screen's summary.
 */
class NotificationsScreen(shell: Shell) : ListPage(shell, "Notifications") {
    private val off = Notifications.onChange { render() }
    private val offWaiting = shell.onWaiting { render() }

    init {
        render()
    }

    private fun render(): Unit = list.rebuild {
        val all = Notifications.all()
        val waiting = shell.waiting
        if (all.isEmpty() && waiting.isEmpty()) {
            note("Nothing new. Reminders you haven't seen, and messages and other notices from your Android apps, show here.")
            return@rebuild
        }
        if (waiting.isNotEmpty()) section("reminders")
        for (w in waiting) {
            val row = LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.TOP
                setPadding(context.dpi(8f), context.dpi(10f), context.dpi(8f), context.dpi(10f))
                addView(android.widget.FrameLayout(context).apply {
                    background = roundRect(Theme.withAlpha(w.color, 0.2f), context.dp(10f))
                    addView(ImageView(context).apply { setImageDrawable(dev.r1os.host.ui.Icons.drawable(w.icon, w.color)) },
                        android.widget.FrameLayout.LayoutParams(context.dpi(20f), context.dpi(20f), Gravity.CENTER))
                }, LinearLayout.LayoutParams(context.dpi(36f), context.dpi(36f)).apply { rightMargin = context.dpi(12f) })
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL
                    addView(context.text(12f, 600, Theme.MUTED).apply { text = "${w.title} · ${ago(w.at)}" })
                    addView(context.text(17f, 700).keepCase().apply {
                        text = w.text.ifEmpty { w.title }
                        maxLines = 3
                        ellipsize = TextUtils.TruncateAt.END
                        setPadding(0, context.dpi(2f), 0, 0)
                    })
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            }
            card(row) { shell.seenWaiting(w, open = true) }
        }
        if (waiting.isNotEmpty() && all.isNotEmpty()) section("android apps")
        for (n in all) {
            val row = LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.TOP
                setPadding(context.dpi(8f), context.dpi(10f), context.dpi(8f), context.dpi(10f))
                addView(ImageView(context).apply { setImageDrawable(n.icon()) },
                    LinearLayout.LayoutParams(context.dpi(36f), context.dpi(36f)).apply { rightMargin = context.dpi(12f) })
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL
                    addView(context.text(12f, 600, Theme.MUTED).apply { text = "${n.app} · ${ago(n.at)}" })
                    addView(context.text(17f, 700).keepCase().apply {
                        text = n.title.ifEmpty { n.app }
                        maxLines = 2
                        ellipsize = TextUtils.TruncateAt.END
                        setPadding(0, context.dpi(2f), 0, 0)
                    })
                    if (n.text.isNotBlank()) addView(context.text(14f, 500, Theme.MUTED).keepCase().apply {
                        text = n.text
                        maxLines = 3
                        ellipsize = TextUtils.TruncateAt.END
                        setPadding(0, context.dpi(2f), 0, 0)
                    })
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            }
            card(row) { shell.openNote(n.key) }
        }
        item("x", "clear all", danger = true) {
            Notifications.clearAll()
            waiting.forEach { shell.seenWaiting(it, open = false) }
        }
    }

    override fun onDestroy() {
        off()
        offWaiting()
    }

    companion object {
        fun ago(at: Long): String {
            val m = (System.currentTimeMillis() - at) / 60_000
            return when {
                m < 1 -> "now"
                m < 60 -> "${m}m"
                m < 24 * 60 -> "${m / 60}h"
                else -> "${m / (24 * 60)}d"
            }
        }
    }
}
