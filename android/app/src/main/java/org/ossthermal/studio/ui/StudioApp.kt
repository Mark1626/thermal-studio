package org.ossthermal.studio.ui

import android.Manifest
import android.bluetooth.BluetoothAdapter
import android.content.Intent
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.roundToInt
import org.ossthermal.studio.LONG_PRINT_MM
import org.ossthermal.studio.StudioViewModel
import org.ossthermal.studio.StudioViewModel.CatalogState
import org.ossthermal.studio.StudioViewModel.PrinterState
import org.ossthermal.studio.StudioViewModel.Rendered
import org.ossthermal.studio.StudioViewModel.Tab
import org.ossthermal.studio.ble.BlePrinter
import uniffi.thermal_ffi.AlgorithmKind
import uniffi.thermal_ffi.Density
import uniffi.thermal_ffi.ImagePreset
import uniffi.thermal_ffi.ImageRotation
import uniffi.thermal_ffi.QrEcc
import uniffi.thermal_ffi.TemplateChoice
import uniffi.thermal_ffi.TemplateField
import uniffi.thermal_ffi.TemplateFieldKind
import uniffi.thermal_ffi.TemplateValue
import uniffi.thermal_ffi.normalizeUrl

private val BLE_PERMISSIONS = arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun StudioApp(vm: StudioViewModel) {
    val context = LocalContext.current
    val snackbar = remember { SnackbarHostState() }
    var showPrinterDialog by remember { mutableStateOf(false) }
    var showPicker by remember { mutableStateOf(false) }
    var confirmLength by remember { mutableStateOf<Float?>(null) }
    var afterPermission by remember { mutableStateOf<(() -> Unit)?>(null) }

    LaunchedEffect(vm.message) {
        vm.message?.let {
            snackbar.showSnackbar(it)
            vm.message = null
        }
    }

    val enableBluetooth = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        if (vm.bluetoothOn) afterPermission?.invoke()
        afterPermission = null
    }
    val requestPermissions = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { granted ->
        if (granted.values.all { it }) {
            if (vm.bluetoothOn) {
                afterPermission?.invoke()
                afterPermission = null
            } else {
                enableBluetooth.launch(Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE))
            }
        } else {
            vm.message = "Bluetooth permission is needed to find the printer"
            afterPermission = null
        }
    }

    /** Run [action] once Bluetooth permissions are granted and Bluetooth is on. */
    fun withBluetooth(action: () -> Unit) {
        val granted = BLE_PERMISSIONS.all {
            ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
        }
        when {
            !granted -> {
                afterPermission = action
                requestPermissions.launch(BLE_PERMISSIONS)
            }
            !vm.bluetoothOn -> {
                afterPermission = action
                enableBluetooth.launch(Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE))
            }
            else -> action()
        }
    }

    fun openPicker() {
        showPrinterDialog = false
        showPicker = true
        withBluetooth(vm::startScan)
    }

    fun onPrint() {
        val length = vm.currentLengthMm ?: return
        when {
            vm.savedPrinter == null -> {
                vm.printAfterChoosing = true
                openPicker()
            }
            length > LONG_PRINT_MM -> confirmLength = length
            else -> withBluetooth(vm::print)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Thermal Studio") },
                actions = {
                    AssistChip(
                        onClick = { showPrinterDialog = true },
                        label = { Text(printerLabel(vm), maxLines = 1) },
                    )
                    Spacer(Modifier.width(8.dp))
                    Button(
                        onClick = ::onPrint,
                        enabled = vm.canPrint,
                        modifier = Modifier.padding(end = 8.dp),
                    ) { Text("Print") }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize()) {
            when (val p = vm.printer) {
                is PrinterState.Printing ->
                    LinearProgressIndicator(progress = { p.fraction }, modifier = Modifier.fillMaxWidth())
                is PrinterState.Busy -> LinearProgressIndicator(Modifier.fillMaxWidth())
                PrinterState.Idle -> Unit
            }
            PrimaryTabRow(selectedTabIndex = vm.tab.ordinal) {
                Tab(selected = vm.tab == Tab.Markdown, onClick = { vm.tab = Tab.Markdown }, text = { Text("Markdown") })
                Tab(selected = vm.tab == Tab.Image, onClick = { vm.tab = Tab.Image }, text = { Text("Image") })
                Tab(selected = vm.tab == Tab.Qr, onClick = { vm.tab = Tab.Qr }, text = { Text("QR code") })
                Tab(selected = vm.tab == Tab.Templates, onClick = { vm.tab = Tab.Templates }, text = { Text("Templates") })
            }
            when (vm.tab) {
                Tab.Markdown -> MarkdownScreen(vm)
                Tab.Image -> ImageScreen(vm)
                Tab.Qr -> QrScreen(vm)
                Tab.Templates -> TemplatesScreen(vm)
            }
        }
    }

    if (showPrinterDialog) {
        PrinterDialog(
            vm,
            onCheck = { withBluetooth(vm::checkStatus) },
            onFindPrinters = ::openPicker,
            onDismiss = { showPrinterDialog = false },
        )
    }
    if (showPicker) {
        PrinterPicker(
            vm,
            onRescan = { withBluetooth(vm::startScan) },
            onChoose = { found ->
                showPicker = false
                withBluetooth { vm.choosePrinter(found) }
            },
            onDismiss = {
                showPicker = false
                vm.stopScan()
                vm.printAfterChoosing = false
            },
        )
    }
    confirmLength?.let { mm ->
        AlertDialog(
            onDismissRequest = { confirmLength = null },
            title = { Text("Long print") },
            text = { Text(String.format(Locale.ROOT, "This will use about %.0f mm of paper.", mm)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmLength = null
                    withBluetooth(vm::print)
                }) { Text("Print") }
            },
            dismissButton = { TextButton(onClick = { confirmLength = null }) { Text("Cancel") } },
        )
    }
}

private fun printerLabel(vm: StudioViewModel): String = when (val state = vm.printer) {
    is PrinterState.Busy -> state.label
    is PrinterState.Printing -> String.format(Locale.ROOT, "%s %.0f%%", state.stage, state.fraction * 100)
    PrinterState.Idle -> {
        val info = vm.lastInfo
        when {
            vm.savedPrinter == null -> "Choose printer"
            info != null -> buildString {
                append(info.model ?: info.name)
                info.battery?.let { append(" · $it%") }
            }
            else -> "Printer"
        }
    }
}

// --- shared -----------------------------------------------------------------------

@Composable
private fun PaperPreview(rendered: Rendered?, placeholder: String, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainerHighest)) {
        if (rendered == null) {
            Text(placeholder, Modifier.align(Alignment.Center).padding(24.dp), color = MaterialTheme.colorScheme.outline)
        } else {
            Column(
                Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Image(
                    bitmap = rendered.bitmap,
                    contentDescription = "Print preview",
                    filterQuality = FilterQuality.None, // keep every dot crisp
                    contentScale = ContentScale.FillWidth,
                    modifier = Modifier
                        .fillMaxWidth()
                        .aspectRatio(rendered.raster.width.toFloat() / rendered.raster.height.coerceAtLeast(1))
                        .border(1.dp, Color.Gray),
                )
                Text(rendered.summary, style = MaterialTheme.typography.labelSmall, modifier = Modifier.padding(top = 6.dp))
            }
        }
    }
}

@Composable
private fun LabeledSlider(
    label: String,
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    onChange: (Float) -> Unit,
    steps: Int = 0,
    format: String = "%.2f",
) {
    Column(Modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(label, style = MaterialTheme.typography.bodyMedium)
            Text(String.format(Locale.ROOT, format, value), style = MaterialTheme.typography.bodyMedium)
        }
        Slider(value = value, onValueChange = onChange, valueRange = range, steps = steps)
    }
}

@Composable
private fun LabeledSwitch(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
        Switch(checked = checked, onCheckedChange = onChange)
    }
}

// --- markdown ---------------------------------------------------------------------

@Composable
private fun MarkdownScreen(vm: StudioViewModel) {
    var preview by remember { mutableStateOf(false) }
    var showStyle by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
            SingleChoiceSegmentedButtonRow(Modifier.weight(1f)) {
                SegmentedButton(
                    selected = !preview,
                    onClick = { preview = false },
                    shape = SegmentedButtonDefaults.itemShape(0, 2),
                ) { Text("Edit") }
                SegmentedButton(
                    selected = preview,
                    onClick = { preview = true },
                    shape = SegmentedButtonDefaults.itemShape(1, 2),
                ) { Text("Preview") }
            }
            TextButton(onClick = { showStyle = !showStyle }) { Text(if (showStyle) "Hide style" else "Style") }
        }
        if (showStyle) MarkdownStyle(vm)
        if (preview) {
            PaperPreview(vm.markdownPreview, "Rendering…", Modifier.weight(1f))
        } else {
            OutlinedTextField(
                value = vm.markdown,
                onValueChange = vm::updateMarkdown,
                modifier = Modifier.fillMaxSize().padding(horizontal = 8.dp).imePadding(),
                textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
            )
        }
    }
}

@Composable
private fun MarkdownStyle(vm: StudioViewModel) {
    val s = vm.markdownSettings
    Column(Modifier.padding(horizontal = 16.dp)) {
        LabeledSlider("Text size (dots)", s.bodySize, 14f..40f, { vm.updateMarkdownSettings(s.copy(bodySize = it)) }, format = "%.0f")
        LabeledSlider("Line height", s.lineHeight, 1f..2f, { vm.updateMarkdownSettings(s.copy(lineHeight = it)) })
        LabeledSlider("Margin (dots)", s.margin.toFloat(), 0f..48f, { vm.updateMarkdownSettings(s.copy(margin = it.toInt())) }, format = "%.0f")
        LabeledSlider(
            "Text boldness",
            (255 - s.textThreshold).toFloat(),
            35f..225f,
            { vm.updateMarkdownSettings(s.copy(textThreshold = 255 - it.toInt())) },
            format = "%.0f",
        )
        HorizontalDivider(Modifier.padding(vertical = 4.dp))
    }
}

// --- image ------------------------------------------------------------------------

@Composable
private fun ImageScreen(vm: StudioViewModel) {
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        uri?.let(vm::openImage)
    }
    val launchPicker = { pick.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }

    if (vm.imageName == null) {
        Column(
            Modifier.fillMaxSize().padding(24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text("Pick a photo, or share one to Thermal Studio from your gallery.", color = MaterialTheme.colorScheme.outline)
            Spacer(Modifier.height(16.dp))
            Button(onClick = launchPicker) { Text("Choose photo") }
        }
        return
    }
    Column(Modifier.fillMaxSize()) {
        PaperPreview(vm.imagePreview, "Rendering…", Modifier.weight(1f))
        HorizontalDivider()
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp)) {
            OutlinedButton(onClick = launchPicker) { Text("Change photo") }
            ImageControls(vm)
        }
    }
}

@Composable
private fun ImageControls(vm: StudioViewModel) {
    val s = vm.imageSettings
    val update = vm::updateImageSettings
    val algorithm = vm.algorithmList.firstOrNull { it.id == s.algorithm }

    Row(Modifier.horizontalScroll(rememberScrollState()).padding(vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf(
            ImagePreset.PHOTO to "Photo",
            ImagePreset.SKETCH to "Sketch",
            ImagePreset.LINE_ART to "Line art",
            ImagePreset.NEUTRAL to "Neutral",
        ).forEach { (preset, label) ->
            FilterChip(selected = false, onClick = { vm.applyPreset(preset) }, label = { Text(label) })
        }
    }

    // Algorithm picker
    var menu by remember { mutableStateOf(false) }
    Box {
        OutlinedButton(onClick = { menu = true }, modifier = Modifier.fillMaxWidth()) {
            Text("Dithering: ${algorithm?.name ?: s.algorithm}")
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            vm.algorithmList.forEach { a ->
                DropdownMenuItem(
                    text = {
                        Column {
                            Text(a.name, fontWeight = if (a.id == s.algorithm) FontWeight.Bold else FontWeight.Normal)
                            Text(a.description, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.outline)
                        }
                    },
                    onClick = {
                        menu = false
                        update(s.copy(algorithm = a.id))
                    },
                )
            }
        }
    }

    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f)) { LabeledSlider("Threshold", s.threshold, 0f..1f, { update(s.copy(threshold = it)) }) }
        TextButton(onClick = vm::autoThreshold) { Text("Auto") }
    }

    when (algorithm?.kind) {
        AlgorithmKind.ERROR_DIFFUSION -> {
            LabeledSlider("Error diffusion", s.diffusion, 0f..1f, { update(s.copy(diffusion = it)) })
            LabeledSwitch("Serpentine scan", s.serpentine) { update(s.copy(serpentine = it)) }
        }
        AlgorithmKind.ORDERED -> if (s.algorithm == "halftone") {
            LabeledSlider("Cell size (dots)", s.halftoneCell.toFloat(), 2f..16f, { update(s.copy(halftoneCell = it.toInt())) }, steps = 13, format = "%.0f")
        }
        AlgorithmKind.STYLIZED -> {
            LabeledSlider("Line width", s.sketchLine, 0.3f..4f, { update(s.copy(sketchLine = it)) })
            LabeledSlider("Edge sensitivity", s.sketchEdges, 0f..1f, { update(s.copy(sketchEdges = it)) })
            LabeledSlider("Shading", s.sketchShading, 0f..1f, { update(s.copy(sketchShading = it)) })
            LabeledSlider("Hatch spacing (dots)", s.sketchHatchSpacing.toFloat(), 3f..16f, { update(s.copy(sketchHatchSpacing = it.toInt())) }, steps = 12, format = "%.0f")
        }
        else -> Unit
    }

    HorizontalDivider(Modifier.padding(vertical = 8.dp))
    LabeledSwitch("Auto levels", s.autoLevels) { update(s.copy(autoLevels = it)) }
    LabeledSlider("Gamma", s.gamma, 0.3f..3f, { update(s.copy(gamma = it)) })
    LabeledSlider("Contrast", s.contrast, -1f..1f, { update(s.copy(contrast = it)) })
    LabeledSlider("Brightness", s.brightness, -1f..1f, { update(s.copy(brightness = it)) })
    LabeledSlider("Sharpen", s.sharpen, 0f..3f, { update(s.copy(sharpen = it)) })
    LabeledSwitch("Invert", s.invert) { update(s.copy(invert = it)) }

    HorizontalDivider(Modifier.padding(vertical = 8.dp))
    LabeledSlider("Width (dots)", s.width.toFloat(), 64f..384f, { update(s.copy(width = (it / 8).toInt() * 8)) }, format = "%.0f")
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf(
            ImageRotation.NONE to "0°",
            ImageRotation.CW90 to "90°",
            ImageRotation.ROT180 to "180°",
            ImageRotation.CCW90 to "270°",
        ).forEach { (r, label) ->
            FilterChip(selected = s.rotation == r, onClick = { update(s.copy(rotation = r)) }, label = { Text(label) })
        }
    }
    Spacer(Modifier.height(24.dp))
}

// --- QR code ----------------------------------------------------------------------

@Composable
private fun QrScreen(vm: StudioViewModel) {
    val placeholder = when {
        vm.qrSettings.url.isBlank() -> "Type a link, or share one to Thermal Studio from your browser."
        else -> vm.qrError ?: "Rendering…"
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        PaperPreview(vm.qrPreview, placeholder, Modifier.weight(1f))
        HorizontalDivider()
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp)) {
            QrControls(vm)
        }
    }
}

@Composable
private fun QrControls(vm: StudioViewModel) {
    val s = vm.qrSettings
    val update = vm::updateQrSettings
    val target = remember(s.url) { normalizeUrl(s.url) }

    OutlinedTextField(
        value = s.url,
        onValueChange = { update(s.copy(url = it)) },
        label = { Text("Link") },
        placeholder = { Text("https://example.com") },
        singleLine = true,
        isError = vm.qrError != null,
        supportingText = if (target.isNotEmpty() && target != s.url.trim()) {
            { Text("Opens $target", maxLines = 2) }
        } else {
            null
        },
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
        modifier = Modifier.fillMaxWidth(),
    )
    OutlinedTextField(
        value = s.caption,
        onValueChange = { update(s.copy(caption = it)) },
        label = { Text("Caption (optional)") },
        singleLine = true,
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Done),
        modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
    )
    LabeledSwitch("Print the link under the code", s.showUrl) { update(s.copy(showUrl = it)) }

    HorizontalDivider(Modifier.padding(vertical = 8.dp))
    LabeledSlider(
        "Largest size (mm)",
        s.maxWidth / 8f,
        16f..48f,
        { update(s.copy(maxWidth = (it * 8).roundToInt())) },
        format = "%.0f",
    )
    Text("Error correction", style = MaterialTheme.typography.bodyMedium)
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        val options = listOf(QrEcc.LOW to "7%", QrEcc.MEDIUM to "15%", QrEcc.QUARTILE to "25%", QrEcc.HIGH to "30%")
        options.forEachIndexed { i, (ecc, label) ->
            SegmentedButton(
                selected = s.ecc == ecc,
                onClick = { update(s.copy(ecc = ecc)) },
                shape = SegmentedButtonDefaults.itemShape(i, options.size),
            ) { Text(label, maxLines = 1) }
        }
    }
    Text(
        "How much of the code can be smudged or creased and still scan. Higher makes a denser code.",
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.outline,
    )
    Spacer(Modifier.height(24.dp))
}

// --- templates --------------------------------------------------------------------

@Composable
private fun TemplatesScreen(vm: StudioViewModel) {
    Column(Modifier.fillMaxSize().imePadding()) {
        PaperPreview(vm.templatePreview, vm.templateError ?: "Rendering…", Modifier.weight(1f))
        HorizontalDivider()
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp)) {
            TemplateControls(vm)
        }
    }
    if (vm.showAddTemplate) AddTemplateDialog(vm)
}

@Composable
private fun TemplateControls(vm: StudioViewModel) {
    val template = vm.template
    val fields = remember(template) { template.fields() }
    val values = vm.templateValues
    val added = vm.addedTemplate
    var menu by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf(false) }
    var confirmEdit by remember { mutableStateOf(false) }

    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f)) {
            OutlinedButton(onClick = { menu = true }, modifier = Modifier.fillMaxWidth()) {
                Text(template.name(), maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                vm.templateList.forEachIndexed { i, t ->
                    if (i == vm.library.builtin.size) HorizontalDivider()
                    DropdownMenuItem(
                        text = {
                            Column {
                                Text(t.name(), fontWeight = if (t.id() == vm.templateId) FontWeight.Bold else FontWeight.Normal)
                                Text(t.description(), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.outline)
                            }
                        },
                        onClick = {
                            menu = false
                            vm.selectTemplate(t.id())
                        },
                    )
                }
            }
        }
        TextButton(onClick = {
            vm.addTemplateError = null
            vm.showAddTemplate = true
        }) { Text("Add…") }
    }
    if (template.description().isNotEmpty()) {
        Text(template.description(), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.outline)
    }
    if (added != null) {
        Text(
            "From ${added.origin.removePrefix("file:")}",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.outline,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 4.dp),
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (added.isLink) {
                TextButton(onClick = vm::refreshTemplate, enabled = !vm.addingTemplate) { Text("Refresh") }
            }
            TextButton(onClick = { confirmRemove = true }) { Text("Remove") }
            if (vm.addingTemplate) CircularProgressIndicator(Modifier.width(18.dp).height(18.dp), strokeWidth = 2.dp)
        }
    }
    HorizontalDivider(Modifier.padding(vertical = 8.dp))

    if (fields.isEmpty()) Text("Nothing to fill in.", color = MaterialTheme.colorScheme.outline)
    fields.forEach { f ->
        TemplateFieldInput(f, values[f.key]) { vm.setTemplateValue(f.key, it) }
    }

    if (vm.templateMissing.isNotEmpty()) {
        Text(
            "Fill in ${vm.templateMissing.joinToString(", ")} to print.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.outline,
            modifier = Modifier.padding(vertical = 4.dp),
        )
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(top = 4.dp)) {
        OutlinedButton(onClick = { if (vm.markdownUntouched) vm.templateToMarkdown() else confirmEdit = true }) {
            Text("Edit as Markdown")
        }
        TextButton(onClick = vm::clearTemplateForm) { Text("Clear form") }
    }
    Spacer(Modifier.height(24.dp))

    if (confirmRemove) {
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text("Remove “${template.name()}”?") },
            text = { Text("You can add it again from its link.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemove = false
                    vm.removeTemplate()
                }) { Text("Remove") }
            },
            dismissButton = { TextButton(onClick = { confirmRemove = false }) { Text("Keep") } },
        )
    }
    if (confirmEdit) {
        AlertDialog(
            onDismissRequest = { confirmEdit = false },
            title = { Text("Replace your note?") },
            text = { Text("This replaces the text in the Markdown tab with this note.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmEdit = false
                    vm.templateToMarkdown()
                }) { Text("Replace") }
            },
            dismissButton = { TextButton(onClick = { confirmEdit = false }) { Text("Cancel") } },
        )
    }
}

/** One input on a template's form, chosen by the field's kind. */
@Composable
private fun TemplateFieldInput(field: TemplateField, value: TemplateValue?, onChange: (TemplateValue) -> Unit) {
    val label = if (field.required) "${field.label} *" else field.label
    val text = (value as? TemplateValue.Text)?.value.orEmpty()
    val placeholder: (@Composable () -> Unit)? = field.placeholder.takeIf { it.isNotEmpty() }?.let { p -> { Text(p) } }
    val help: (@Composable () -> Unit)? = field.help.takeIf { it.isNotEmpty() }?.let { h -> { Text(h) } }
    val modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp)
    when (val kind = field.kind) {
        // No auto-capitals or autocorrect: they would quietly change passwords and addresses.
        TemplateFieldKind.Text -> OutlinedTextField(
            value = text,
            onValueChange = { onChange(TemplateValue.Text(it)) },
            label = { Text(label) },
            placeholder = placeholder,
            supportingText = help,
            singleLine = true,
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, imeAction = ImeAction.Next),
            modifier = modifier,
        )
        TemplateFieldKind.Multiline -> OutlinedTextField(
            value = text,
            onValueChange = { onChange(TemplateValue.Text(it)) },
            label = { Text(label) },
            placeholder = placeholder,
            supportingText = help,
            minLines = 3,
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
            modifier = modifier,
        )
        is TemplateFieldKind.Choice -> ChoiceInput(label, kind.options, text, onChange)
        TemplateFieldKind.Switch -> LabeledSwitch(label, (value as? TemplateValue.Switch)?.on ?: false) {
            onChange(TemplateValue.Switch(it))
        }
        TemplateFieldKind.Entries -> EntriesInput(label, field.placeholder, (value as? TemplateValue.Entries)?.items.orEmpty(), onChange)
        TemplateFieldKind.Date -> DateInput(label, text, field.required, onChange)
    }
    // Text boxes show their help inside; other inputs show it underneath.
    val helpInside = field.kind == TemplateFieldKind.Text || field.kind == TemplateFieldKind.Multiline
    if (field.help.isNotEmpty() && !helpInside) {
        Text(field.help, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.outline)
    }
    Spacer(Modifier.height(6.dp))
}

@Composable
private fun ChoiceInput(label: String, options: List<TemplateChoice>, value: String, onChange: (TemplateValue) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val shown = options.firstOrNull { it.value == value }?.label ?: value
    Box {
        OutlinedButton(onClick = { open = true }, modifier = Modifier.fillMaxWidth()) {
            Text("$label: $shown", maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            options.forEach { o ->
                DropdownMenuItem(
                    text = { Text(o.label, fontWeight = if (o.value == value) FontWeight.Bold else FontWeight.Normal) },
                    onClick = {
                        open = false
                        onChange(TemplateValue.Text(o.value))
                    },
                )
            }
        }
    }
}

/** A text box per entry with a remove button, and a button to add one. */
@Composable
private fun EntriesInput(label: String, placeholder: String, items: List<String>, onChange: (TemplateValue) -> Unit) {
    Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 4.dp))
    items.forEachIndexed { i, item ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(
                value = item,
                onValueChange = { new -> onChange(TemplateValue.Entries(items.toMutableList().also { it[i] = new })) },
                placeholder = placeholder.takeIf { it.isNotEmpty() }?.let { p -> { Text(p) } },
                singleLine = true,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Next),
                modifier = Modifier.weight(1f).padding(vertical = 2.dp),
            )
            TextButton(onClick = { onChange(TemplateValue.Entries(items.filterIndexed { j, _ -> j != i })) }) {
                Text("×", style = MaterialTheme.typography.titleLarge)
            }
        }
    }
    TextButton(onClick = { onChange(TemplateValue.Entries(items + "")) }) { Text("+ Add") }
}

private const val DAY_MS = 86_400_000L

/** A date picked from a calendar. Templates get it as `2026-11-05`. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DateInput(label: String, iso: String, required: Boolean, onChange: (TemplateValue) -> Unit) {
    var picking by remember { mutableStateOf(false) }
    val date = remember(iso) { runCatching { LocalDate.parse(iso) }.getOrNull() }
    val shown = date?.format(DateTimeFormatter.ofPattern("EEEE d MMMM yyyy", Locale.getDefault())) ?: "No date"
    Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 4.dp))
    Row(verticalAlignment = Alignment.CenterVertically) {
        OutlinedButton(onClick = { picking = true }, modifier = Modifier.weight(1f)) { Text(shown) }
        if (date != null && !required) {
            TextButton(onClick = { onChange(TemplateValue.Text("")) }) { Text("Clear") }
        }
    }
    if (picking) {
        // The picker works in UTC milliseconds; a LocalDate maps to midnight UTC and back.
        val state = rememberDatePickerState(initialSelectedDateMillis = (date ?: LocalDate.now()).toEpochDay() * DAY_MS)
        DatePickerDialog(
            onDismissRequest = { picking = false },
            confirmButton = {
                TextButton(onClick = {
                    state.selectedDateMillis?.let {
                        onChange(TemplateValue.Text(LocalDate.ofEpochDay(Math.floorDiv(it, DAY_MS)).toString()))
                    }
                    picking = false
                }) { Text("OK") }
            },
            dismissButton = { TextButton(onClick = { picking = false }) { Text("Cancel") } },
        ) { DatePicker(state) }
    }
}

@Composable
private fun AddTemplateDialog(vm: StudioViewModel) {
    var link by rememberSaveable { mutableStateOf("") }
    val pickFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        uri?.let(vm::addTemplateFromFile)
    }
    val busy = vm.addingTemplate
    LaunchedEffect(Unit) { vm.loadCatalog() }
    AlertDialog(
        onDismissRequest = { if (!busy) vm.showAddTemplate = false },
        title = { Text("Add a template") },
        text = {
            Column(Modifier.heightIn(max = 520.dp).verticalScroll(rememberScrollState())) {
                Text("Official templates", style = MaterialTheme.typography.titleSmall)
                when (val c = vm.catalog) {
                    null, CatalogState.Loading -> Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.padding(vertical = 8.dp),
                    ) {
                        CircularProgressIndicator(Modifier.width(18.dp).height(18.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(8.dp))
                        Text("Loading the list…", color = MaterialTheme.colorScheme.outline)
                    }
                    is CatalogState.Failed -> {
                        Text("Couldn't load the list. ${c.message}", color = MaterialTheme.colorScheme.error)
                        TextButton(onClick = vm::loadCatalog) { Text("Try again") }
                    }
                    is CatalogState.Ready -> {
                        if (c.entries.isEmpty()) {
                            Text("No more templates yet.", color = MaterialTheme.colorScheme.outline)
                        }
                        c.entries.forEach { entry ->
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 2.dp)) {
                                Column(Modifier.weight(1f)) {
                                    Text(entry.name, fontWeight = FontWeight.Bold)
                                    if (entry.description.isNotEmpty()) {
                                        Text(
                                            entry.description,
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.outline,
                                        )
                                    }
                                }
                                val action = vm.catalogAction(entry)
                                if (action == null) {
                                    Text(
                                        "Added",
                                        style = MaterialTheme.typography.labelLarge,
                                        color = MaterialTheme.colorScheme.outline,
                                        modifier = Modifier.padding(horizontal = 12.dp),
                                    )
                                } else {
                                    TextButton(onClick = { vm.installFromCatalog(entry) }, enabled = !busy) { Text(action) }
                                }
                            }
                        }
                    }
                }
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
                Text("Or add one from a link", style = MaterialTheme.typography.titleSmall)
                OutlinedTextField(
                    value = link,
                    onValueChange = {
                        link = it
                        vm.addTemplateError = null
                    },
                    label = { Text("Link") },
                    placeholder = { Text("https://…/template.md") },
                    singleLine = true,
                    enabled = !busy,
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Uri,
                        autoCorrectEnabled = false,
                        imeAction = ImeAction.Go,
                    ),
                    keyboardActions = KeyboardActions(onGo = { if (link.isNotBlank()) vm.addTemplateFromLink(link) }),
                    modifier = Modifier.fillMaxWidth(),
                )
                if (busy) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 8.dp)) {
                        CircularProgressIndicator(Modifier.width(18.dp).height(18.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(8.dp))
                        Text("Adding…")
                    }
                }
                TextButton(
                    onClick = { pickFile.launch(arrayOf("text/*", "application/octet-stream")) },
                    enabled = !busy,
                ) { Text("Open a file instead") }
                vm.addTemplateError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                Text(
                    "A template is a Markdown file with a few blanks to fill in. It can't run programs or reach the network.",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.outline,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { vm.addTemplateFromLink(link) }, enabled = link.isNotBlank() && !busy) { Text("Add link") }
        },
        dismissButton = {
            TextButton(onClick = { vm.showAddTemplate = false }, enabled = !busy) { Text("Cancel") }
        },
    )
}

// --- printer ----------------------------------------------------------------------

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PrinterDialog(
    vm: StudioViewModel,
    onCheck: () -> Unit,
    onFindPrinters: () -> Unit,
    onDismiss: () -> Unit,
) {
    val saved = vm.savedPrinter
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Printer") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                if (saved == null) {
                    Text("No printer chosen yet.")
                    Spacer(Modifier.height(8.dp))
                    Button(onClick = onFindPrinters) { Text("Find printers") }
                } else {
                    Text(saved.name, fontWeight = FontWeight.Bold)
                    when (val p = vm.printer) {
                        is PrinterState.Busy -> Text(p.label)
                        is PrinterState.Printing -> Text("Printing…")
                        PrinterState.Idle -> vm.lastInfo?.let { i ->
                            Text("Model ${i.model ?: "?"} · firmware ${i.firmware ?: "?"}")
                            Text("Battery ${i.battery?.let { "$it%" } ?: "?"} · ${i.status?.description ?: "status unknown"}")
                        }
                    }
                    Spacer(Modifier.height(4.dp))
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        OutlinedButton(onClick = onCheck, enabled = !vm.busy) { Text("Check status") }
                        OutlinedButton(onClick = onFindPrinters, enabled = !vm.busy) { Text("Change") }
                        OutlinedButton(onClick = vm::forgetPrinter, enabled = !vm.busy) { Text("Forget") }
                    }
                }
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
                Text("Density", style = MaterialTheme.typography.bodyMedium)
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                    val options = listOf(null to "Default", Density.LIGHT to "Light", Density.NORMAL to "Normal", Density.DARK to "Dark")
                    options.forEachIndexed { i, (d, label) ->
                        SegmentedButton(
                            selected = vm.density == d,
                            onClick = { vm.density = d },
                            shape = SegmentedButtonDefaults.itemShape(i, options.size),
                        ) { Text(label, maxLines = 1) }
                    }
                }
                LabeledSlider("Feed after print (mm)", vm.feedMm, 0f..30f, { vm.feedMm = it }, format = "%.0f")
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )
}

/** "Choose printer": printers found by a live search; tap one to remember it. */
@Composable
private fun PrinterPicker(
    vm: StudioViewModel,
    onRescan: () -> Unit,
    onChoose: (BlePrinter.Found) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Choose printer") },
        text = {
            Column {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (vm.scanning) {
                        CircularProgressIndicator(Modifier.width(18.dp).height(18.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(8.dp))
                        Text("Searching…", Modifier.weight(1f))
                    } else {
                        Text("Search finished", Modifier.weight(1f))
                        TextButton(onClick = onRescan) { Text("Search again") }
                    }
                }
                HorizontalDivider()
                Column(Modifier.heightIn(max = 320.dp).verticalScroll(rememberScrollState())) {
                    if (vm.scanResults.isEmpty() && !vm.scanning) {
                        Text(
                            "No printers found. Switch the printer on, close its phone app, and search again.",
                            Modifier.padding(vertical = 12.dp),
                            color = MaterialTheme.colorScheme.outline,
                        )
                    }
                    vm.scanResults.forEach { d ->
                        Text(
                            d.name,
                            Modifier.fillMaxWidth().clickable { onChoose(d) }.padding(vertical = 14.dp),
                            style = MaterialTheme.typography.bodyLarge,
                        )
                        HorizontalDivider()
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
