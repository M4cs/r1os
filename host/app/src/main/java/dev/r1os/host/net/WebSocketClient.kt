package dev.r1os.host.net

import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.IOException
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.security.SecureRandom
import java.util.Base64
import kotlin.concurrent.thread

/**
 * A minimal WebSocket client (RFC 6455) for r1d's `/ws` on loopback: it reads text frames and
 * answers pings. r1d only pushes to the shell (actions go over HTTP), so it never sends text.
 * No extensions, no TLS: the peer is r1d on 127.0.0.1 (ADR-0017).
 */
class WebSocketClient(
    private val host: String,
    private val port: Int,
    private val path: String,
    /** r1d's per-boot token (ADR-0026), sent with the handshake. */
    private val token: String?,
    private val onOpen: () -> Unit,
    /** Called on the socket's own thread, so parsing stays off the main thread. */
    private val onText: (String) -> Unit,
    private val onClose: () -> Unit,
) {
    @Volatile private var socket: Socket? = null
    @Volatile private var closed = false
    private val random = SecureRandom()

    fun connect() {
        thread(name = "r1d-ws", isDaemon = true) { run() }
    }

    fun close() {
        closed = true
        runCatching { socket?.close() }
    }

    private fun run() {
        try {
            val s = Socket()
            s.tcpNoDelay = true
            s.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
            socket = s
            val out = s.getOutputStream()
            val key = Base64.getEncoder().encodeToString(ByteArray(16).also { random.nextBytes(it) })
            out.write(
                ("GET $path HTTP/1.1\r\nHost: $host:$port\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
                    (if (token != null) "Authorization: Bearer $token\r\n" else "") +
                    "Sec-WebSocket-Key: $key\r\nSec-WebSocket-Version: 13\r\n\r\n").toByteArray(),
            )
            out.flush()
            val input = DataInputStream(BufferedInputStream(s.getInputStream()))
            val status = readLine(input)
            if (!status.contains(" 101")) throw IOException("WebSocket handshake failed: $status")
            while (readLine(input).isNotEmpty()) Unit // the rest of the headers
            onOpen()
            val message = ByteArrayOutputStream()
            while (!closed) {
                val b0 = input.readUnsignedByte()
                val b1 = input.readUnsignedByte()
                val fin = b0 and 0x80 != 0
                val opcode = b0 and 0x0f
                var len = (b1 and 0x7f).toLong()
                if (len == 126L) len = input.readUnsignedShort().toLong() else if (len == 127L) len = input.readLong()
                if (len > MAX_FRAME) throw IOException("frame too large: $len")
                val mask = if (b1 and 0x80 != 0) ByteArray(4).also { input.readFully(it) } else null
                val data = ByteArray(len.toInt())
                input.readFully(data)
                if (mask != null) for (i in data.indices) data[i] = (data[i].toInt() xor mask[i % 4].toInt()).toByte()
                when (opcode) {
                    OP_CONTINUATION, OP_TEXT -> {
                        message.write(data)
                        if (fin) {
                            onText(message.toString("UTF-8"))
                            message.reset()
                        }
                    }
                    OP_CLOSE -> break
                    OP_PING -> send(out, OP_PONG, data)
                    else -> Unit
                }
            }
        } catch (_: Exception) {
            // Closed, refused (r1d not up yet) or broken: the owner reconnects.
        } finally {
            runCatching { socket?.close() }
            socket = null
            onClose()
        }
    }

    /** Client frames must be masked (RFC 6455 §5.3). */
    private fun send(out: OutputStream, opcode: Int, payload: ByteArray) {
        val mask = ByteArray(4).also { random.nextBytes(it) }
        val frame = ByteArrayOutputStream()
        frame.write(0x80 or opcode)
        when {
            payload.size < 126 -> frame.write(0x80 or payload.size)
            payload.size < 65536 -> {
                frame.write(0x80 or 126)
                frame.write(payload.size shr 8)
                frame.write(payload.size and 0xff)
            }
            else -> {
                frame.write(0x80 or 127)
                for (i in 7 downTo 0) frame.write(((payload.size.toLong() shr (8 * i)) and 0xff).toInt())
            }
        }
        frame.write(mask)
        for (i in payload.indices) frame.write(payload[i].toInt() xor mask[i % 4].toInt())
        synchronized(out) {
            out.write(frame.toByteArray())
            out.flush()
        }
    }

    private fun readLine(input: DataInputStream): String {
        val sb = StringBuilder()
        while (true) {
            val c = input.readUnsignedByte()
            if (c == '\n'.code) break
            if (c != '\r'.code) sb.append(c.toChar())
        }
        return sb.toString()
    }

    private companion object {
        const val CONNECT_TIMEOUT_MS = 3000
        const val MAX_FRAME = 8L * 1024 * 1024
        const val OP_CONTINUATION = 0x0
        const val OP_TEXT = 0x1
        const val OP_CLOSE = 0x8
        const val OP_PING = 0x9
        const val OP_PONG = 0xA
    }
}
