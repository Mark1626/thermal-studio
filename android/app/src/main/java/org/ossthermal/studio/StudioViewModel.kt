package org.ossthermal.studio

import android.app.Application
import android.content.Context
import android.net.Uri
import android.util.Log
import androidx.core.content.edit
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.time.LocalDate
import java.util.Locale
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.ossthermal.studio.ble.BlePrinter
import org.ossthermal.studio.ble.PrinterException
import uniffi.thermal_ffi.AlgorithmInfo
import uniffi.thermal_ffi.CatalogEntry
import uniffi.thermal_ffi.Density
import uniffi.thermal_ffi.ImagePreset
import uniffi.thermal_ffi.ImageSettings
import uniffi.thermal_ffi.ImageSource
import uniffi.thermal_ffi.MarkdownRenderer
import uniffi.thermal_ffi.MarkdownSettings
import uniffi.thermal_ffi.NoteTemplate
import uniffi.thermal_ffi.QrSettings
import uniffi.thermal_ffi.Raster
import uniffi.thermal_ffi.TemplateValue
import uniffi.thermal_ffi.algorithms
import uniffi.thermal_ffi.defaultMarkdownSettings
import uniffi.thermal_ffi.defaultQrSettings
import uniffi.thermal_ffi.imagePreset
import uniffi.thermal_ffi.templateLink

/** Ask before printing anything longer than this. */
const val LONG_PRINT_MM = 200f

private val SAMPLE = """
    # Welcome to Thermal Studio

    Type your note here, then tap **Preview** to see exactly what will print.

    - **Bold**, *italic* and ~~strikethrough~~
    - Headings, lists, quotes and tables

    Tamil works too: வணக்கம்! தமிழிலும் அச்சிடலாம்.

    > To print a photo or a link, share it to Thermal Studio, or use the **Image** and **QR code** tabs.
    > For a Wi-Fi card, a checklist or a label, try **Templates**.
""".trimIndent()

/** A shared text that is nothing but a web link. */
private val WEB_LINK = Regex("""https?://\S+""", RegexOption.IGNORE_CASE)

class StudioViewModel(app: Application) : AndroidViewModel(app) {

    enum class Tab { Markdown, Image, Qr, Templates }

    data class Rendered(val raster: Raster, val bitmap: ImageBitmap, val millis: Long) {
        val summary: String
            get() = String.format(
                Locale.ROOT,
                "%d × %d dots · %.1f mm · %.0f%% ink · %d ms",
                raster.width, raster.height, raster.lengthMm, raster.inkRatio * 100, millis,
            )
    }

    /** The printer is only connected while a job or status check runs. */
    sealed interface PrinterState {
        data object Idle : PrinterState
        data class Busy(val label: String) : PrinterState
        data class Printing(val stage: String, val fraction: Float) : PrinterState
    }

    var tab by mutableStateOf(Tab.Markdown)
    var message by mutableStateOf<String?>(null)

    // --- markdown ----------------------------------------------------------------

    var markdown by mutableStateOf(SAMPLE)
        private set
    var markdownSettings by mutableStateOf(defaultMarkdownSettings())
        private set
    var markdownPreview by mutableStateOf<Rendered?>(null)
        private set

    /** Shared by Markdown and QR rendering (fonts load once); `lazy` so both can start it safely. */
    private val rendererLazy = lazy { MarkdownRenderer() }
    private val renderer by rendererLazy
    private var markdownJob: Job? = null

    fun updateMarkdown(text: String) {
        markdown = text
        renderMarkdown(debounceMs = 150)
    }

    fun updateMarkdownSettings(settings: MarkdownSettings) {
        markdownSettings = settings
        renderMarkdown(debounceMs = 50)
    }

    private fun renderMarkdown(debounceMs: Long) {
        markdownJob?.cancel()
        markdownJob = viewModelScope.launch {
            delay(debounceMs)
            val (text, settings) = markdown to markdownSettings
            markdownPreview = runCatchingRender {
                withContext(Dispatchers.Default) {
                    timed { renderer.render(text, settings, imagesDir()) }
                }
            } ?: markdownPreview
        }
    }

    private fun imagesDir(): String = getApplication<Application>().filesDir.resolve("images").path

    // --- image -------------------------------------------------------------------

    val algorithmList: List<AlgorithmInfo> = algorithms()
    var imageName by mutableStateOf<String?>(null)
        private set
    var imageSettings by mutableStateOf(imagePreset(ImagePreset.PHOTO))
        private set
    var imagePreview by mutableStateOf<Rendered?>(null)
        private set

    private var imageSource: ImageSource? = null
    private var imageJob: Job? = null

    fun openImage(uri: Uri) {
        tab = Tab.Image
        viewModelScope.launch {
            try {
                val source = withContext(Dispatchers.Default) { decodeImageSource(getApplication(), uri) }
                imageSource?.close()
                imageSource = source
                imageName = uri.lastPathSegment ?: "image"
                renderImage(debounceMs = 0)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                message = "Couldn't open image: ${e.message}"
            }
        }
    }

    fun updateImageSettings(settings: ImageSettings) {
        imageSettings = settings
        renderImage(debounceMs = 40)
    }

    fun applyPreset(preset: ImagePreset) {
        updateImageSettings(imagePreset(preset).copy(width = imageSettings.width, rotation = imageSettings.rotation))
    }

    fun autoThreshold() {
        val source = imageSource ?: return
        viewModelScope.launch {
            val t = withContext(Dispatchers.Default) { source.autoThreshold(imageSettings) }
            updateImageSettings(imageSettings.copy(threshold = t))
        }
    }

    private fun renderImage(debounceMs: Long) {
        val source = imageSource ?: return
        imageJob?.cancel()
        imageJob = viewModelScope.launch {
            delay(debounceMs)
            val settings = imageSettings
            imagePreview = runCatchingRender {
                withContext(Dispatchers.Default) { timed { source.render(settings) } }
            } ?: imagePreview
        }
    }

    /**
     * Shared text: a lone link becomes a QR code, captioned with [subject] (browsers send the page
     * title); anything else is edited as Markdown.
     */
    fun shareText(text: String, subject: String? = null) {
        val link = text.trim()
        if (WEB_LINK.matches(link)) {
            val caption = subject?.trim()?.takeUnless { it == link }.orEmpty()
            openLink(link, caption)
        } else {
            tab = Tab.Markdown
            updateMarkdown(text)
        }
    }

    // --- QR code -----------------------------------------------------------------

    var qrSettings by mutableStateOf(defaultQrSettings())
        private set
    var qrPreview by mutableStateOf<Rendered?>(null)
        private set

    /** Why the current link can't be drawn (e.g. too long to fit). */
    var qrError by mutableStateOf<String?>(null)
        private set

    private var qrJob: Job? = null

    fun updateQrSettings(settings: QrSettings) {
        qrSettings = settings
        renderQr(debounceMs = 60)
    }

    fun openLink(url: String, caption: String = "") {
        tab = Tab.Qr
        qrSettings = qrSettings.copy(url = url, caption = caption)
        renderQr(debounceMs = 0)
    }

    private fun renderQr(debounceMs: Long) {
        qrJob?.cancel()
        val settings = qrSettings
        if (settings.url.isBlank()) {
            qrPreview = null
            qrError = null
            return
        }
        qrJob = viewModelScope.launch {
            delay(debounceMs)
            try {
                qrPreview = withContext(Dispatchers.Default) { timed { renderer.renderQr(settings) } }
                qrError = null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Never leave an outdated code on screen where Print would send it.
                qrPreview = null
                qrError = "Can't make a QR code: ${e.message}"
            }
        }
    }

    // --- templates ---------------------------------------------------------------

    val library = TemplateLibrary(app)

    /** Built-in templates first, then added ones. */
    var templateList by mutableStateOf(library.all)
        private set

    private val templatePrefs = app.getSharedPreferences("templates", Context.MODE_PRIVATE)

    var templateId by mutableStateOf(
        templatePrefs.getString("selected", null)?.takeIf { library.find(it) != null }
            ?: library.builtin.first().id(),
    )
        private set

    val template: NoteTemplate get() = library.find(templateId) ?: library.builtin.first()

    /** Where the current template came from, if it was added rather than built in. */
    val addedTemplate: TemplateLibrary.Added? get() = library.addedById(templateId)

    /** What has been typed into each template's form this session. Not saved: forms can hold passwords. */
    private val typed = mutableStateMapOf<String, Map<String, TemplateValue>>()

    val templateValues: Map<String, TemplateValue>
        get() = typed[templateId] ?: template.defaults(today())

    var templatePreview by mutableStateOf<Rendered?>(null)
        private set

    /** Why the note can't be drawn: a half-typed date, or a broken template. */
    var templateError by mutableStateOf<String?>(null)
        private set

    /** Labels of required fields still empty; printing waits for them. */
    var templateMissing by mutableStateOf<List<String>>(emptyList())
        private set

    /** The official list of templates, loaded when the "Add a template" dialog first opens. */
    sealed interface CatalogState {
        data object Loading : CatalogState
        data class Ready(val entries: List<CatalogEntry>) : CatalogState
        data class Failed(val message: String) : CatalogState
    }

    var catalog by mutableStateOf<CatalogState?>(null)
        private set

    /** Load the official list once (or again after a failure). */
    fun loadCatalog() {
        if (catalog is CatalogState.Loading || catalog is CatalogState.Ready) return
        catalog = CatalogState.Loading
        viewModelScope.launch {
            catalog = try {
                CatalogState.Ready(withContext(Dispatchers.IO) { downloadCatalog() })
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                CatalogState.Failed(e.message ?: e.toString())
            }
        }
    }

    /** The list's button for [entry]: "Add", "Update", or null when it's added and up to date. */
    fun catalogAction(entry: CatalogEntry): String? {
        val have = library.addedById(entry.id) ?: return "Add"
        return if (have.template.version() != entry.version) "Update" else null
    }

    fun installFromCatalog(entry: CatalogEntry) =
        addTemplate(refresh = library.addedById(entry.id) != null) { entry.url to downloadTemplate(entry.url) }

    /** The "Add a template" dialog: shown, downloading, and why the last try failed. */
    var showAddTemplate by mutableStateOf(false)
    var addingTemplate by mutableStateOf(false)
        private set
    var addTemplateError by mutableStateOf<String?>(null)

    private var templateJob: Job? = null

    fun selectTemplate(id: String) {
        templateId = id
        templatePrefs.edit { putString("selected", id) }
        renderTemplate(debounceMs = 0)
    }

    fun setTemplateValue(key: String, value: TemplateValue) {
        typed[templateId] = templateValues + (key to value)
        renderTemplate(debounceMs = 150)
    }

    fun clearTemplateForm() {
        typed.remove(templateId)
        renderTemplate(debounceMs = 0)
    }

    private fun renderTemplate(debounceMs: Long) {
        templateJob?.cancel()
        val (t, values, today) = Triple(template, templateValues, today())
        templateMissing = try {
            t.missing(values, today)
        } catch (e: Exception) {
            emptyList()
        }
        templateJob = viewModelScope.launch {
            delay(debounceMs)
            try {
                templatePreview = withContext(Dispatchers.Default) {
                    timed { renderer.render(t.fill(values, today), defaultMarkdownSettings(), null) }
                }
                templateError = null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Never leave an outdated note on screen where Print would send it.
                templatePreview = null
                templateError = e.message ?: e.toString()
            }
        }
    }

    fun addTemplateFromLink(input: String) {
        val link = try {
            templateLink(input)
        } catch (e: Exception) {
            addTemplateError = e.message
            return
        }
        addTemplate { link to downloadTemplate(link) }
    }

    fun addTemplateFromFile(uri: Uri) = addTemplate {
        val (name, text) = readTemplateFile(getApplication(), uri)
        "file:$name" to text
    }

    /** Download an added template again from its link. */
    fun refreshTemplate() {
        val link = addedTemplate?.takeIf { it.isLink }?.origin ?: return
        addTemplate(refresh = true) { link to downloadTemplate(link) }
    }

    private fun addTemplate(refresh: Boolean = false, fetch: () -> Pair<String, String>) {
        addingTemplate = true
        addTemplateError = null
        viewModelScope.launch {
            try {
                val t = withContext(Dispatchers.IO) {
                    val (origin, source) = fetch()
                    library.add(origin, source)
                }
                templateList = library.all
                typed.remove(t.id()) // its fields may have changed
                showAddTemplate = false
                message = if (refresh) "Updated “${t.name()}”" else "Added “${t.name()}”"
                selectTemplate(t.id())
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val why = e.message ?: e.toString()
                if (showAddTemplate) addTemplateError = why else message = "Couldn't update the template: $why"
            } finally {
                addingTemplate = false
            }
        }
    }

    fun removeTemplate() {
        val id = addedTemplate?.template?.id() ?: return
        try {
            library.remove(id)
        } catch (e: Exception) {
            message = e.message
            return
        }
        typed.remove(id)
        templateList = library.all
        selectTemplate(library.builtin.first().id())
    }

    /** True if the Markdown tab holds nothing worth keeping (so it can be replaced without asking). */
    val markdownUntouched: Boolean get() = markdown == SAMPLE || markdown.isBlank()

    /** Copy the filled-in note to the Markdown tab, to change it freely. */
    fun templateToMarkdown() {
        try {
            updateMarkdown(template.fill(templateValues, today()))
            tab = Tab.Markdown
        } catch (e: Exception) {
            message = e.message
        }
    }

    // --- printing ----------------------------------------------------------------

    data class SavedPrinter(val address: String, val name: String)

    var printer by mutableStateOf<PrinterState>(PrinterState.Idle)
        private set

    /** Model, battery and status from the last connection (kept after disconnecting). */
    var lastInfo by mutableStateOf<BlePrinter.Info?>(null)
        private set
    var density by mutableStateOf<Density?>(null)
    var feedMm by mutableStateOf(10f)

    private val prefs = app.getSharedPreferences("printer", Context.MODE_PRIVATE)

    /** The printer chosen from the list, remembered across launches. */
    var savedPrinter by mutableStateOf(loadSavedPrinter())
        private set

    private fun loadSavedPrinter(): SavedPrinter? {
        val address = prefs.getString("address", null) ?: return null
        return SavedPrinter(address, prefs.getString("name", null) ?: address)
    }

    private fun savePrinter(p: SavedPrinter?) {
        prefs.edit {
            if (p == null) clear() else putString("address", p.address).putString("name", p.name)
        }
        savedPrinter = p
    }

    private val ble by lazy { BlePrinter(getApplication()) }
    private val printerMutex = Mutex()

    val bluetoothOn: Boolean get() = ble.isBluetoothOn
    val busy: Boolean get() = printer != PrinterState.Idle

    val currentPreview: Rendered?
        get() = when (tab) {
            Tab.Markdown -> markdownPreview
            Tab.Image -> imagePreview
            Tab.Qr -> qrPreview
            Tab.Templates -> templatePreview
        }

    /** Something is ready to print, and (for a template) its required fields are filled in. */
    val canPrint: Boolean
        get() = currentPreview != null && !busy && !(tab == Tab.Templates && templateMissing.isNotEmpty())

    /** Paper the current preview would use, including the feed after printing. */
    val currentLengthMm: Float? get() = currentPreview?.let { it.raster.lengthMm + feedMm }

    // --- choosing a printer --------------------------------------------------------

    /** Printers found by the current or last search, in the order they appeared. */
    var scanResults by mutableStateOf<List<BlePrinter.Found>>(emptyList())
        private set
    var scanning by mutableStateOf(false)
        private set

    /** Print as soon as a printer has been chosen (set when Print is pressed with none chosen). */
    var printAfterChoosing = false

    private var scanJob: Job? = null

    fun startScan() {
        scanJob?.cancel()
        scanResults = emptyList()
        scanning = true
        scanJob = viewModelScope.launch {
            try {
                withTimeoutOrNull(12_000) {
                    ble.scan().collect { found -> scanResults = scanResults + found }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                message = e.message
            } finally {
                scanning = false
            }
        }
    }

    fun stopScan() {
        scanJob?.cancel()
        scanJob = null
        scanning = false
    }

    fun choosePrinter(found: BlePrinter.Found) {
        stopScan()
        savePrinter(SavedPrinter(found.address, found.name))
        lastInfo = null
        if (printAfterChoosing) {
            printAfterChoosing = false
            print()
        } else {
            checkStatus()
        }
    }

    fun forgetPrinter() {
        savePrinter(null)
        lastInfo = null
    }

    // --- printing ------------------------------------------------------------------

    /** Connect, read model/battery/status, disconnect. */
    fun checkStatus() = printerTask("check status") {
        withConnection { lastInfo = ble.info() }
    }

    fun print() {
        if (tab == Tab.Templates && templateMissing.isNotEmpty()) return
        val raster = currentPreview?.raster ?: return
        printerTask("print") {
            withConnection {
                printer = PrinterState.Printing("starting", 0f)
                val report = ble.print(raster, density, feedMm) { p ->
                    printer = PrinterState.Printing(p.stage, p.sent.toFloat() / p.total.coerceAtLeast(1))
                }
                message = String.format(
                    Locale.ROOT,
                    "Printed %.1f mm (%.1f KB)%s",
                    raster.lengthMm, report.bytes / 1024f,
                    report.statusAfter?.let { " · printer ${it.description}" } ?: "",
                )
                lastInfo = ble.info()
            }
        }
    }

    /**
     * Connect to the saved printer, run [block], always disconnect afterwards. Like the
     * vendor app, the printer is only held while there's work to do, so it's free for
     * other devices and its LED goes back to idle.
     */
    private suspend fun withConnection(block: suspend () -> Unit) {
        val saved = savedPrinter ?: throw PrinterException("Choose a printer first")
        printer = PrinterState.Busy("Connecting…")
        try {
            val address = ble.connect(saved.address, saved.name)
            if (address != saved.address) savePrinter(saved.copy(address = address))
            block()
        } finally {
            ble.disconnect()
        }
    }

    private fun printerTask(name: String, block: suspend () -> Unit) {
        viewModelScope.launch {
            printerMutex.withLock {
                Log.d("ThermalStudio", "$name: start")
                try {
                    block()
                    Log.d("ThermalStudio", "$name: done")
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.d("ThermalStudio", "$name: failed: $e")
                    message = e.message ?: e.toString()
                } finally {
                    printer = PrinterState.Idle
                }
            }
        }
    }

    // --- helpers -----------------------------------------------------------------

    private inline fun timed(block: () -> Raster): Rendered {
        val start = System.nanoTime()
        val raster = block()
        return Rendered(raster, raster.toImageBitmap(), (System.nanoTime() - start) / 1_000_000)
    }

    /** The local date, as templates expect it: `2026-11-05`. */
    private fun today(): String = LocalDate.now().toString()

    private suspend fun runCatchingRender(block: suspend () -> Rendered): Rendered? = try {
        block()
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        message = "Render failed: ${e.message}"
        null
    }

    init {
        renderMarkdown(debounceMs = 0)
        renderTemplate(debounceMs = 0)
    }

    override fun onCleared() {
        stopScan()
        ble.disconnect()
        if (rendererLazy.isInitialized()) renderer.close()
        imageSource?.close()
    }
}
