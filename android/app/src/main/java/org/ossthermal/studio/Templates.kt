package org.ossthermal.studio

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Log
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URL
import java.net.UnknownHostException
import org.json.JSONArray
import org.json.JSONObject
import uniffi.thermal_ffi.CatalogEntry
import uniffi.thermal_ffi.NoteTemplate
import uniffi.thermal_ffi.builtinTemplates
import uniffi.thermal_ffi.maxIndexBytes
import uniffi.thermal_ffi.maxTemplateBytes
import uniffi.thermal_ffi.officialIndexUrl
import uniffi.thermal_ffi.parseTemplateIndex

/**
 * The built-in templates plus the ones added from a link or a file. Added templates are
 * kept exactly as downloaded (in `templates.json`), so they work offline.
 */
class TemplateLibrary(context: Context) {

    /** A template the user added. [origin] is an `https://` link, or `file:` plus a file name. */
    class Added(val origin: String, val source: String, val template: NoteTemplate) {
        val isLink: Boolean get() = origin.startsWith("https://") || origin.startsWith("http://")
    }

    val builtin: List<NoteTemplate> = builtinTemplates()

    var added: List<Added> = emptyList()
        private set

    /** Saved templates this version can't read (made for a newer one, say); kept so they return after an update. */
    private var unreadable: List<JSONObject> = emptyList()

    private val file = File(context.filesDir, "templates.json")

    init {
        load()
    }

    val all: List<NoteTemplate> get() = builtin + added.map { it.template }

    fun find(id: String): NoteTemplate? = all.firstOrNull { it.id() == id }

    fun addedById(id: String): Added? = added.firstOrNull { it.template.id() == id }

    /**
     * Add a template, or update the added one with the same id. Throws with a message for
     * the user if it isn't a valid template or would replace a built-in one.
     */
    fun add(origin: String, source: String): NoteTemplate {
        val template = NoteTemplate.parse(source)
        val id = template.id()
        if (builtin.any { it.id() == id }) {
            throw IOException("Its id, `$id`, is already used by a built-in template.")
        }
        added = added.filterNot { it.template.id() == id } + Added(origin, source, template)
        save()
        return template
    }

    fun remove(id: String) {
        added = added.filterNot { it.template.id() == id }
        save()
    }

    private fun load() {
        if (!file.exists()) return
        val keep = mutableListOf<JSONObject>()
        val loaded = mutableListOf<Added>()
        try {
            val array = JSONArray(file.readText())
            for (i in 0 until array.length()) {
                val entry = array.getJSONObject(i)
                val origin = entry.getString("origin")
                val source = entry.getString("source")
                try {
                    loaded += Added(origin, source, NoteTemplate.parse(source))
                } catch (e: Exception) {
                    keep += entry
                }
            }
        } catch (e: Exception) {
            Log.w("ThermalStudio", "couldn't read saved templates", e)
        }
        added = loaded
        unreadable = keep
    }

    private fun save() {
        val array = JSONArray()
        added.forEach { array.put(JSONObject().put("origin", it.origin).put("source", it.source)) }
        unreadable.forEach { array.put(it) }
        // Write a new file and swap it in, so a crash can't leave half a file behind.
        val tmp = File(file.parentFile, "${file.name}.tmp")
        tmp.writeText(array.toString())
        if (!tmp.renameTo(file)) throw IOException("Couldn't save the template")
    }
}

/** Download a template over HTTPS (call off the main thread). Errors carry a message for the user. */
fun downloadTemplate(link: String): String = download(link, maxTemplateBytes())

/** The templates the official list offers (call off the main thread). */
fun downloadCatalog(): List<CatalogEntry> {
    val url = officialIndexUrl()
    return parseTemplateIndex(download(url, maxIndexBytes()), url)
}

/** Download at most [limit] bytes of text. */
private fun download(link: String, limit: Int): String {
    val conn = URL(link).openConnection() as HttpURLConnection
    conn.connectTimeout = 15_000
    conn.readTimeout = 15_000
    conn.instanceFollowRedirects = true // Android never follows a redirect from https to http
    conn.setRequestProperty("User-Agent", "thermal-studio")
    try {
        val code = conn.responseCode
        when {
            code == 404 -> throw IOException("Nothing was found at that link.")
            code !in 200..299 -> throw IOException("The server refused (HTTP $code).")
        }
        return conn.inputStream.use { readLimited(it, limit) }
    } catch (e: UnknownHostException) {
        throw IOException("Couldn't find that server. Check the link and your connection.")
    } catch (e: SocketTimeoutException) {
        throw IOException("The server took too long to answer.")
    } finally {
        conn.disconnect()
    }
}

/** Read a template file the user picked (call off the main thread). Returns its name and text. */
fun readTemplateFile(context: Context, uri: Uri): Pair<String, String> {
    val name = context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
        ?: uri.lastPathSegment
        ?: "file"
    val text = context.contentResolver.openInputStream(uri)?.use { readLimited(it, maxTemplateBytes()) }
        ?: throw IOException("Couldn't open that file.")
    return name to text
}

/** Read at most [limit] bytes of UTF-8 text. */
private fun readLimited(input: InputStream, limit: Int): String {
    val bytes = input.readNBytes(limit + 1)
    if (bytes.size > limit) throw IOException("That file is too large (the limit is ${limit / 1024} KB).")
    return bytes.toString(Charsets.UTF_8)
}
