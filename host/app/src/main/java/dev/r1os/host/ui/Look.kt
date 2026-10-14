package dev.r1os.host.ui

import android.content.Context
import org.json.JSONObject

/**
 * The device's look (ADR-0043), as r1d keeps it (crate::look): dark or light, the accent, the font, each screen's
 * background, and the glance screen's clock and layout. Kept here too, so the shell starts in it before r1d answers.
 * [update] says when it changed: views take their colours when they're made, so the shell is then built again.
 */
object Look {
    private const val PREFS = "look"
    var json = JSONObject()
        private set

    /** The look as last seen (before any view is made). */
    fun load(context: Context) {
        val s = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString("look", null)
        json = s?.let { runCatching { JSONObject(it) }.getOrNull() } ?: JSONObject()
        applyTheme()
    }

    /** A look from r1d. True when it isn't the one in use: the theme has it now, and the shell must be built again. */
    fun update(context: Context, look: JSONObject): Boolean {
        if (look.toString() == json.toString()) return false
        json = look
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString("look", look.toString()).apply()
        applyTheme()
        return true
    }

    private fun applyTheme() =
        Theme.setLook(json.optString("mode") == "light", json.optString("accent", "orange"), json.optString("font", "rubik"))

    private val glance: JSONObject get() = json.optJSONObject("glance") ?: JSONObject()

    /** The glance screen's clock face (crate::look CLOCKS). */
    val clock: String get() = glance.optString("clock", "bold")
    /** Where its time and status sit: left or center. */
    val align: String get() = glance.optString("align", "left")
    /** Where its agent peeks up: center, left, right or hidden. */
    val agentPlace: String get() = glance.optString("agent", "center")

    /** Whether the glance screen leaves this part out (date, status, alarm, notifications, playing, widgets). */
    fun hides(part: String): Boolean {
        val a = glance.optJSONArray("hide") ?: return false
        return (0 until a.length()).any { a.optString(it) == part }
    }

    /** A screen's background (`glance` or `home`): `{kind, color?, style?, photo?, dim?}`. */
    fun background(screen: String): JSONObject =
        json.optJSONObject(screen)?.optJSONObject("background") ?: JSONObject().put("kind", "plain")
}
