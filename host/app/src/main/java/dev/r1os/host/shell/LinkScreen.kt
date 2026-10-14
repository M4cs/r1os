package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.net.Uri
import android.view.View
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import java.io.ByteArrayInputStream
import java.net.InetAddress

/**
 * A web page an app opened (its `open_url` step, which r1d checked: the app holds `open.url` and the link
 * is on the web). The R1 has no browser, so this is one: a WebView under the page's host name. The wheel
 * scrolls it, and touch works as anywhere. One tick up at the top lights Back, as in an app, and a double
 * click goes back.
 *
 * It has no bridge to r1os, and it loads nothing from the R1 or the network it's on (r1d, the pairing page,
 * your router), only http(s) on the web. Pages keep their cookies like any browser's.
 */
@SuppressLint("SetJavaScriptEnabled")
class LinkScreen(shell: Shell, url: String) : Screen(shell) {
    private val header = BackHeader(context, Uri.parse(url).host?.removePrefix("www.") ?: "link")
    override val backHeader get() = header
    private val web = WebView(context)
    override val view = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundColor(Theme.BG)
    }

    init {
        header.onBackTouched { shell.back() }
        web.apply {
            setBackgroundColor(android.graphics.Color.WHITE)
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.setSupportMultipleWindows(false)
            settings.javaScriptCanOpenWindowsAutomatically = false
            webViewClient = object : WebViewClient() {
                // Links within the web stay here; anything else (another app's scheme, the R1 itself) doesn't open.
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean = !onTheWeb(request.url)

                // Nothing on the R1 or its network, even from a page's scripts and images.
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
                    if (onTheWeb(request.url)) null else WebResourceResponse("text/plain", "utf-8", 403, "Forbidden", emptyMap(), ByteArrayInputStream(ByteArray(0)))

                override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
                    Uri.parse(url).host?.let { header.setTitle(it.removePrefix("www.")) }
                }
            }
        }
        view.addView(header, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(14f - BackHeader.INSET), context.dpi(10f), context.dpi(14f), context.dpi(6f))
        })
        view.addView(FrameLayout(context).apply {
            addView(web, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        if (onTheWeb(Uri.parse(url))) web.loadUrl(url)
    }

    override fun onEvent(ev: Ev): Boolean {
        if (ev.type == "wheel") {
            val d = ev.int("delta")
            when {
                // Back is lit: down puts it out, up stays.
                header.shown -> if (d > 0) header.hide()
                d < 0 && !web.canScrollVertically(-1) -> header.show()
                else -> web.scrollBy(0, d * web.height / 3)
            }
            return true
        }
        if (ev.isTap && header.shown) {
            shell.back()
            return true
        }
        return false
    }

    override fun onShow() {
        web.onResume()
        KeptPages.showing(this, true, web)
    }

    override fun onHide() {
        web.onPause()
        KeptPages.showing(this, false, web)
    }

    override fun onDestroy() {
        KeptPages.showing(this, false, web)
        web.stopLoading()
        (web.parent as? android.view.ViewGroup)?.removeView(web)
        web.destroy()
    }

    private companion object {
        /** http(s) to a host on the web: not the R1, nor a name or address on its network. */
        fun onTheWeb(uri: Uri): Boolean {
            if (uri.scheme != "https" && uri.scheme != "http") return false
            val host = uri.host?.lowercase()?.trimEnd('.') ?: return false
            if (host == "localhost" || host.endsWith(".localhost") || host.endsWith(".local") || host.endsWith(".internal")) return false
            // An address written out: judged as one (a name is left to DNS, as in any browser).
            val literal = host.removePrefix("[").removeSuffix("]")
            if (literal.all { it.isDigit() || it == '.' } || literal.contains(':')) {
                val ip = runCatching { InetAddress.getByName(literal) }.getOrNull() ?: return false
                return !(ip.isLoopbackAddress || ip.isSiteLocalAddress || ip.isLinkLocalAddress || ip.isAnyLocalAddress ||
                    ip.isMulticastAddress || (ip.address.size == 16 && (ip.address[0].toInt() and 0xfe) == 0xfc))
            }
            return host.contains('.')
        }
    }
}
