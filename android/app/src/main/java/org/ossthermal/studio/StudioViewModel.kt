package org.ossthermal.studio

import android.app.Application
import android.content.Context
import android.net.Uri
import android.util.Log
import androidx.core.content.edit
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
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
import uniffi.thermal_ffi.Density
import uniffi.thermal_ffi.ImagePreset
import uniffi.thermal_ffi.ImageSettings
import uniffi.thermal_ffi.ImageSource
import uniffi.thermal_ffi.MarkdownRenderer
import uniffi.thermal_ffi.MarkdownSettings
import uniffi.thermal_ffi.Raster
import uniffi.thermal_ffi.algorithms
import uniffi.thermal_ffi.defaultMarkdownSettings
import uniffi.thermal_ffi.imagePreset

/** Ask before printing anything longer than this. */
const val LONG_PRINT_MM = 200f

private val SAMPLE = """
    # Thermal notes

    Printed from **Android** on a *58 mm* roll.

    - [x] Rust core via UniFFI
    - [ ] Print something fun

    வணக்கம்! தமிழிலும் அச்சிடலாம்.

    > Share a photo to this app, or open the Image tab.
""".trimIndent()

class StudioViewModel(app: Application) : AndroidViewModel(app) {

    enum class Tab { Markdown, Image }

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

    private var renderer: MarkdownRenderer? = null
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
                    val r = renderer ?: MarkdownRenderer().also { renderer = it }
                    timed { r.render(text, settings, imagesDir()) }
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

    fun shareText(text: String) {
        tab = Tab.Markdown
        updateMarkdown(text)
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
        get() = if (tab == Tab.Markdown) markdownPreview else imagePreview

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
    }

    override fun onCleared() {
        stopScan()
        ble.disconnect()
        renderer?.close()
        imageSource?.close()
    }
}
