package dev.r1os.host

import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.graphics.drawable.Drawable
import android.os.Handler
import android.os.Looper
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import dev.r1os.host.net.R1d
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.CopyOnWriteArraySet

/**
 * Android apps' notifications (a Signal message, a delivery), shown in r1os: the R1 has no status bar or shade
 * (ADR-0014), so without this they'd be invisible. [NotificationListener] hears them; here they're kept, shown (a banner
 * while the screen is on, the glance screen's summary, the launcher's Notifications), opened (their app comes up where
 * it was) and cleared. r1d gets them too, for the agents ("any messages?").
 *
 * What's kept: what an app would alert you with or list. Not r1os's own, ongoing ones (a download, music playing:
 * the glance's now-playing has that), group summaries, or silent ones.
 */
object Notifications {
    private const val TAG = "r1os"
    private val main = Handler(Looper.getMainLooper())

    class Item(
        val key: String,
        val pkg: String,
        val app: String,
        val title: String,
        val text: String,
        val at: Long,
        /** It would make a sound or show at the top on a phone: r1os shows a banner for it. */
        val alerting: Boolean,
        private val intent: PendingIntent?,
        /** Its reply action (a messaging app's), when it has one that takes words. */
        private val reply: Notification.Action?,
        private val context: Context,
    ) {
        val canReply get() = reply != null

        /** Replies with `text`, as typing in the notification's reply box would. */
        fun reply(text: String): Boolean = runCatching {
            val a = reply ?: return false
            val inputs = a.remoteInputs ?: return false
            val results = android.os.Bundle()
            for (ri in inputs) if (ri.allowFreeFormInput) results.putCharSequence(ri.resultKey, text)
            val fill = android.content.Intent().addFlags(android.content.Intent.FLAG_RECEIVER_FOREGROUND)
            android.app.RemoteInput.addResultsToIntent(inputs, fill, results)
            a.actionIntent.send(context, 0, fill)
            true
        }.onFailure { Log.w(TAG, "notification: reply $pkg", it) }.getOrDefault(false)
        fun icon(): Drawable? = runCatching { context.packageManager.getApplicationIcon(pkg) }.getOrNull()

        fun open(): Boolean = runCatching {
            val i = intent ?: return AndroidApps.open(context, pkg)
            // Android 14 starts another app's activity from a PendingIntent only when the sender says so.
            val opts = android.app.ActivityOptions.makeBasic()
            if (android.os.Build.VERSION.SDK_INT >= 34) {
                opts.setPendingIntentBackgroundActivityStartMode(android.app.ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
            }
            i.send(context, 0, null, null, null, null, opts.toBundle())
            true
        }.onFailure { Log.w(TAG, "notification: open $pkg", it) }.getOrDefault(false)

        fun json(): JSONObject =
            JSONObject().put("key", key).put("package", pkg).put("app", app).put("title", title).put("text", text).put("at", at).put("can_reply", canReply)
    }

    private val items = LinkedHashMap<String, Item>()
    private var service: NotificationListenerService? = null
    private val posted = CopyOnWriteArraySet<(Item) -> Unit>()
    private val changed = CopyOnWriteArraySet<() -> Unit>()

    /** Newest first. */
    fun all(): List<Item> = items.values.sortedByDescending { it.at }

    val count get() = items.size

    /** A new one to show (on the main thread). */
    fun onPosted(fn: (Item) -> Unit): () -> Unit {
        posted.add(fn)
        return { posted.remove(fn) }
    }

    /** The list changed. */
    fun onChange(fn: () -> Unit): () -> Unit {
        changed.add(fn)
        return { changed.remove(fn) }
    }

    fun open(key: String): Boolean {
        val item = items[key] ?: return false
        val ok = item.open()
        // Opened is seen: it goes, as tapping it on a phone would (unless the app keeps it).
        if (ok) dismiss(key)
        return ok
    }

    fun dismiss(key: String) {
        runCatching { service?.cancelNotification(key) }
        remove(key)
    }

    fun clearAll() {
        runCatching { service?.cancelAllNotifications() }
        val keys = items.keys.toList()
        keys.forEach { remove(it) }
    }

    /** r1d asks (an agent's `open_notification`, `dismiss_notification`, `reply_notification`). */
    fun onEvent(op: String?, key: String?, text: String?) {
        key ?: return
        when (op) {
            "open" -> open(key)
            "dismiss" -> dismiss(key)
            "reply" -> items[key]?.let { item -> if (!text.isNullOrBlank() && item.reply(text)) { /* the app updates or removes it */ } }
        }
    }

    // ---- from the listener -------------------------------------------------------------------------

    fun connected(s: NotificationListenerService) = main.post {
        service = s
        items.clear()
        val ranking = runCatching { s.currentRanking }.getOrNull()
        for (sbn in runCatching { s.activeNotifications }.getOrNull().orEmpty()) {
            read(s, sbn, ranking)?.let { items[it.key] = it }
        }
        changed.forEach { it() }
        publishAll()
    }

    fun disconnected() = main.post { service = null }

    fun posted(s: NotificationListenerService, sbn: StatusBarNotification, ranking: NotificationListenerService.RankingMap?) = main.post {
        val item = read(s, sbn, ranking)
        if (item == null) {
            if (items.containsKey(sbn.key)) remove(sbn.key)
            return@post
        }
        val known = items[item.key]
        items[item.key] = item
        changed.forEach { it() }
        // An update to one already here (a download's progress, the same message again) isn't news.
        if (item.alerting && (known == null || known.text != item.text || known.title != item.title)) posted.forEach { it(item) }
        R1d.post("/api/android/notifications", JSONObject().put("op", "posted").put("note", item.json()), onError = {})
    }

    fun removed(sbn: StatusBarNotification) = main.post { remove(sbn.key) }

    private fun remove(key: String) {
        if (items.remove(key) == null) return
        changed.forEach { it() }
        R1d.post("/api/android/notifications", JSONObject().put("op", "removed").put("key", key), onError = {})
    }

    /** All of them to r1d (it keeps them in memory: after its restart too, when the shell reconnects). */
    fun publishAll() {
        val notes = JSONArray()
        all().forEach { notes.put(it.json()) }
        R1d.post("/api/android/notifications", JSONObject().put("op", "reset").put("notes", notes), onError = { Log.w(TAG, "notifications: $it") })
    }

    private fun read(s: NotificationListenerService, sbn: StatusBarNotification, ranking: NotificationListenerService.RankingMap?): Item? {
        val n = sbn.notification
        if (sbn.packageName == s.packageName) return null
        if (sbn.isOngoing || n.flags and Notification.FLAG_FOREGROUND_SERVICE != 0) return null
        if (n.flags and Notification.FLAG_GROUP_SUMMARY != 0) return null
        if (n.category == Notification.CATEGORY_TRANSPORT || n.category == Notification.CATEGORY_SERVICE) return null
        val extras = n.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val text = (extras.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: extras.getCharSequence(Notification.EXTRA_TEXT))?.toString().orEmpty()
        if (title.isBlank() && text.isBlank()) return null
        val r = NotificationListenerService.Ranking()
        val importance = if (ranking?.getRanking(sbn.key, r) == true) r.importance else android.app.NotificationManager.IMPORTANCE_DEFAULT
        if (importance < android.app.NotificationManager.IMPORTANCE_LOW) return null
        val pm = s.packageManager
        val app = runCatching { pm.getApplicationLabel(pm.getApplicationInfo(sbn.packageName, 0)).toString() }.getOrDefault(sbn.packageName)
        return Item(
            key = sbn.key,
            pkg = sbn.packageName,
            app = app,
            title = title.trim(),
            text = text.trim().take(400),
            at = sbn.postTime,
            alerting = importance >= android.app.NotificationManager.IMPORTANCE_DEFAULT,
            intent = n.contentIntent,
            reply = n.actions?.firstOrNull { a -> a.remoteInputs?.any { it.allowFreeFormInput } == true },
            context = s.applicationContext,
        )
    }
}

/** Android's door for [Notifications]: the system binds it once the host may read notifications. */
class NotificationListener : NotificationListenerService() {
    override fun onListenerConnected() = Notifications.connected(this).let { }
    override fun onListenerDisconnected() = Notifications.disconnected().let { }
    override fun onNotificationPosted(sbn: StatusBarNotification, rankingMap: RankingMap?) = Notifications.posted(this, sbn, rankingMap).let { }
    override fun onNotificationRemoved(sbn: StatusBarNotification) = Notifications.removed(sbn).let { }
}
