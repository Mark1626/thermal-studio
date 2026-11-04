package org.ossthermal.studio.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.SystemClock
import android.util.Log
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.thermal_ffi.Density
import uniffi.thermal_ffi.JobSettings
import uniffi.thermal_ffi.PrintReport
import uniffi.thermal_ffi.PrintSession
import uniffi.thermal_ffi.PrinterStatus
import uniffi.thermal_ffi.Query
import uniffi.thermal_ffi.Raster
import uniffi.thermal_ffi.SessionAction
import uniffi.thermal_ffi.SessionProgress
import uniffi.thermal_ffi.gattInfo
import uniffi.thermal_ffi.looksLikePrinter
import uniffi.thermal_ffi.parseBattery
import uniffi.thermal_ffi.parseStatus
import uniffi.thermal_ffi.parseText
import uniffi.thermal_ffi.queryCommand

class PrinterException(message: String) : Exception(message)

private const val TAG = "ThermalStudio"
private fun log(msg: String) = Log.d(TAG, msg)

/**
 * BLE client for LuckPrinter-family printers (service FF00: write FF02, replies FF01,
 * credits FF03). The print flow itself lives in Rust ([PrintSession]); this class only
 * performs the I/O the session asks for.
 *
 * Android allows one outstanding GATT operation at a time, so callers must not run
 * operations concurrently (the view model serialises them with a mutex).
 */
@SuppressLint("MissingPermission") // Callers request BLUETOOTH_SCAN / BLUETOOTH_CONNECT first.
class BlePrinter(private val context: Context) {

    data class Info(
        val name: String,
        val model: String?,
        val firmware: String?,
        val battery: Int?,
        val status: PrinterStatus?,
    )

    /** A printer seen while scanning. */
    data class Found(val address: String, val name: String)

    private val ids = gattInfo()
    private val serviceUuid = UUID.fromString(ids.service)
    private val writeUuid = UUID.fromString(ids.write)
    private val replyUuid = UUID.fromString(ids.reply)
    private val flowUuid = UUID.fromString(ids.flow)
    private val cccdUuid = UUID.fromString(ids.cccd)

    private val manager = context.getSystemService(BluetoothManager::class.java)
    private var gatt: BluetoothGatt? = null
    private var writeChar: BluetoothGattCharacteristic? = null
    private val replies = Channel<ByteArray>(Channel.UNLIMITED)
    /** Devices from the latest scans, so a chosen one can be connected without rescanning. */
    private val seen = ConcurrentHashMap<String, BluetoothDevice>()

    @Volatile private var pending: CompletableDeferred<Int>? = null
    @Volatile private var flowListener: ((ByteArray) -> Unit)? = null
    @Volatile private var maxWriteLen = 20

    @Volatile var isConnected = false
        private set
    var deviceName: String = ""
        private set

    val isBluetoothOn: Boolean get() = manager?.adapter?.isEnabled == true

    private val callback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
            log("connection state: newState=$newState status=$status")
            if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
                isConnected = true
                pending?.complete(status)
            } else {
                isConnected = false
                pending?.completeExceptionally(PrinterException("Printer disconnected (status $status)"))
            }
        }

        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS) maxWriteLen = mtu - 3
            pending?.complete(status)
        }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            pending?.complete(status)
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) {
            pending?.complete(status)
        }

        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            pending?.complete(status)
        }

        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
            when (c.uuid) {
                replyUuid -> replies.trySend(value)
                flowUuid -> flowListener?.invoke(value)
            }
        }
    }

    /** Start one GATT operation and wait for its callback. */
    private suspend fun gattOp(timeoutMs: Long, what: String, start: () -> Boolean): Int {
        val done = CompletableDeferred<Int>()
        pending = done
        try {
            if (!start()) {
                log("$what: could not start")
                throw PrinterException("Could not start $what")
            }
            return withTimeoutOrNull(timeoutMs) { done.await() }
                ?: run {
                    log("$what: timed out after $timeoutMs ms")
                    throw PrinterException("Timed out during $what")
                }
        } finally {
            pending = null
        }
    }

    private val scanSettings: ScanSettings =
        ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()

    /** A [Found] if [result] looks like a supported printer, else `null`. */
    private fun printerOf(result: ScanResult): Found? {
        val name = result.scanRecord?.deviceName ?: result.device.name ?: return null
        val advertisesService = result.scanRecord?.serviceUuids?.any { it.uuid == serviceUuid } == true
        return if (advertisesService || looksLikePrinter(name)) Found(result.device.address, name) else null
    }

    /**
     * Nearby printers for the "Choose printer" list, each reported once. Scans until the
     * flow's collector is cancelled. The printer doesn't advertise its service UUID, so
     * this scans unfiltered and recognises printers by name.
     */
    fun scan(): Flow<Found> = callbackFlow {
        val scanner = manager?.adapter?.bluetoothLeScanner ?: throw PrinterException("Bluetooth is off")
        val cb = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val found = printerOf(result) ?: return
                if (seen.put(found.address, result.device) == null) {
                    log("found ${found.name} (${found.address})")
                    trySend(found)
                }
            }

            override fun onScanFailed(errorCode: Int) {
                close(PrinterException("Bluetooth scan failed (error $errorCode)"))
            }
        }
        seen.clear()
        log("scan started")
        scanner.startScan(null, scanSettings, cb)
        awaitClose {
            scanner.stopScan(cb)
            log("scan stopped")
        }
    }

    /** Scan for a device advertising exactly [name]. */
    private suspend fun findByName(name: String, timeoutMs: Long): BluetoothDevice {
        val scanner = manager?.adapter?.bluetoothLeScanner ?: throw PrinterException("Bluetooth is off")
        val hit = CompletableDeferred<BluetoothDevice>()
        val cb = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val found = printerOf(result) ?: return
                if (found.name.equals(name, ignoreCase = true)) {
                    seen[found.address] = result.device
                    hit.complete(result.device)
                }
            }

            override fun onScanFailed(errorCode: Int) {
                hit.completeExceptionally(PrinterException("Bluetooth scan failed (error $errorCode)"))
            }
        }
        scanner.startScan(null, scanSettings, cb)
        try {
            return withTimeoutOrNull(timeoutMs) { hit.await() }
                ?: throw PrinterException("“$name” not found. Is it switched on and not connected to another app?")
        } finally {
            scanner.stopScan(cb)
        }
    }

    /**
     * Connect to the chosen printer. Tries its saved [address] directly (no scan); if that
     * fails — the printer may have changed its BLE address, or Android forgot it — finds
     * it again by [name]. Returns the address actually connected, which the caller should
     * save if it changed.
     */
    suspend fun connect(address: String, name: String): String {
        disconnect()
        deviceName = name
        val adapter = manager?.adapter ?: throw PrinterException("Bluetooth is off")
        val direct = seen[address] ?: adapter.getRemoteDevice(address)
        try {
            log("connecting to $address")
            open(direct, connectTimeoutMs = 8_000)
            log("connected to $address (max write $maxWriteLen bytes)")
            return address
        } catch (e: PrinterException) {
            log("direct connection failed: ${e.message}; searching for “$name”")
            disconnect()
        }
        val device = findByName(name, timeoutMs = 10_000)
        open(device, connectTimeoutMs = 15_000)
        log("connected to ${device.address} after search")
        return device.address
    }

    /** Connect, negotiate MTU and subscribe to replies and credits. */
    private suspend fun open(device: BluetoothDevice, connectTimeoutMs: Long) {
        try {
            gattOp(connectTimeoutMs, "connection") {
                // Deprecated in API 37, but its replacement doesn't exist on Android 13–16.
                @Suppress("DEPRECATION")
                gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
                gatt != null
            }
            val g = gatt!!
            // Default ATT MTU (23) only fits 20-byte writes; the printer negotiates ~240.
            gattOp(5_000, "MTU negotiation") { g.requestMtu(247) }
            g.requestConnectionPriority(BluetoothGatt.CONNECTION_PRIORITY_HIGH)
            gattOp(10_000, "service discovery") { g.discoverServices() }
            val service = g.getService(serviceUuid) ?: throw PrinterException("Not a supported printer (no FF00 service)")
            writeChar = service.getCharacteristic(writeUuid) ?: throw PrinterException("Characteristic FF02 missing")
            for (uuid in listOf(replyUuid, flowUuid)) {
                val c = service.getCharacteristic(uuid) ?: throw PrinterException("Characteristic $uuid missing")
                g.setCharacteristicNotification(c, true)
                val cccd = c.getDescriptor(cccdUuid) ?: throw PrinterException("Notification descriptor missing")
                gattOp(5_000, "enabling notifications") {
                    g.writeDescriptor(cccd, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE) ==
                        BluetoothStatusCodes.SUCCESS
                }
            }
            delay(300) // let connect-time frames arrive, then discard them
            drain()
        } catch (e: Exception) {
            disconnect()
            throw e
        }
    }

    fun disconnect() {
        gatt?.let {
            log("disconnecting")
            it.disconnect()
            it.close()
        }
        gatt = null
        writeChar = null
        isConnected = false
    }

    private fun drain() {
        while (replies.tryReceive().isSuccess) {
            // discard stale notifications
        }
    }

    private suspend fun write(data: ByteArray) {
        val g = gatt ?: throw PrinterException("Not connected")
        val c = writeChar ?: throw PrinterException("Not connected")
        val status = gattOp(5_000, "write") {
            g.writeCharacteristic(c, data, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT) ==
                BluetoothStatusCodes.SUCCESS
        }
        if (status != BluetoothGatt.GATT_SUCCESS) throw PrinterException("Write failed (GATT status $status)")
    }

    private suspend fun query(q: Query): ByteArray? {
        drain()
        write(queryCommand(q))
        return withTimeoutOrNull(2_000) { replies.receive() }
    }

    suspend fun info(): Info = Info(
        name = deviceName,
        model = query(Query.MODEL)?.let(::parseText)?.ifBlank { null },
        firmware = query(Query.FIRMWARE)?.let(::parseText)?.ifBlank { null },
        battery = query(Query.BATTERY)?.let(::parseBattery),
        status = query(Query.STATUS)?.let(::parseStatus),
    )

    /** Print a 1-bit raster, reporting progress as the session advances. */
    suspend fun print(
        raster: Raster,
        density: Density?,
        feedMm: Float,
        onProgress: (SessionProgress) -> Unit,
    ): PrintReport {
        PrintSession(raster, JobSettings(density, feedMm, maxWriteLen)).use { session ->
            flowListener = { session.onFlow(it) }
            try {
                val start = SystemClock.elapsedRealtime()
                fun now() = SystemClock.elapsedRealtime() - start
                log("print: ${raster.width}x${raster.height}, chunks of ≤$maxWriteLen bytes")
                onProgress(session.progress())
                var stage = ""
                while (true) {
                    val p = session.progress()
                    if (p.stage != stage) {
                        stage = p.stage
                        log("print: stage $stage (${p.sent}/${p.total} bytes) at ${now()} ms")
                    }
                    when (val action = session.poll(now())) {
                        SessionAction.DrainReplies -> drain()
                        is SessionAction.Write -> {
                            write(action.data)
                            session.onWriteDone()
                            onProgress(session.progress())
                        }
                        is SessionAction.Sleep -> delay((action.untilMs - now()).coerceAtLeast(0))
                        is SessionAction.AwaitReply -> {
                            val wait = (action.deadlineMs - now()).coerceAtLeast(1)
                            val reply = withTimeoutOrNull(wait) { replies.receive() }
                            if (reply != null) session.onReply(reply) else session.onTimeout()
                            onProgress(session.progress())
                        }
                        SessionAction.AwaitWrite -> error("writes are reported before polling")
                        is SessionAction.Finished -> {
                            log("print: finished in ${now()} ms, acknowledged=${action.report.acknowledged}")
                            return action.report
                        }
                        is SessionAction.Failed -> {
                            log("print: failed at ${now()} ms: ${action.message}")
                            throw PrinterException(action.message)
                        }
                    }
                }
            } finally {
                flowListener = null
            }
        }
    }
}
