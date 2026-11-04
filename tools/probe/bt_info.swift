import Foundation
import IOBluetooth

guard CommandLine.arguments.count > 1 else {
    print("usage: bt_info <address>   (e.g. 01-23-45-67-89-ab)"); exit(1)
}
let addr = CommandLine.arguments[1]
guard let dev = IOBluetoothDevice(addressString: addr) else {
    print("no device for \(addr)"); exit(1)
}

func hex(_ v: UInt32, _ w: Int = 6) -> String { String(format: "0x%0\(w)X", v) }

print("name:              \(dev.name ?? "nil")")
print("nameOrAddress:     \(dev.nameOrAddress ?? "nil")")
print("address:           \(dev.addressString ?? "nil")")
print("classOfDevice:     \(hex(dev.classOfDevice))")
print("  majorClass:      \(hex(dev.deviceClassMajor, 2))")
print("  minorClass:      \(hex(dev.deviceClassMinor, 2))")
print("  serviceClass:    \(hex(dev.serviceClassMajor, 4))")
print("isPaired:          \(dev.isPaired())")
print("isConnected:       \(dev.isConnected())")
print("lastInquiryUpdate: \(String(describing: dev.getLastInquiryUpdate()))")
print("lastServicesUpdate:\(String(describing: dev.getLastServicesUpdate()))")

class SDPDelegate: NSObject {
    var done = false
    @objc func sdpQueryComplete(_ device: IOBluetoothDevice!, status: IOReturn) {
        print("\nSDP query complete, status=\(String(format: "0x%08X", status))")
        done = true
    }
}

func dumpServices(_ dev: IOBluetoothDevice) {
    guard let services = dev.services as? [IOBluetoothSDPServiceRecord], !services.isEmpty else {
        print("(no cached SDP service records)"); return
    }
    for rec in services {
        print("\n== Service: \(rec.getServiceName() ?? "<unnamed>")")
        var ch: BluetoothRFCOMMChannelID = 0
        if rec.getRFCOMMChannelID(&ch) == kIOReturnSuccess { print("   RFCOMM channel: \(ch)") }
        var psm: BluetoothL2CAPPSM = 0
        if rec.getL2CAPPSM(&psm) == kIOReturnSuccess { print("   L2CAP PSM: \(psm)") }
        if let attrs = rec.attributes as? [NSNumber: IOBluetoothSDPDataElement] {
            for (k, v) in attrs.sorted(by: { $0.key.intValue < $1.key.intValue }) {
                print(String(format: "   attr 0x%04X: ", k.intValue) + describe(v))
            }
        }
    }
}

func describe(_ e: IOBluetoothSDPDataElement) -> String {
    let t = e.getTypeDescriptor()
    switch Int(t) {
    case kBluetoothSDPDataElementTypeUUID:
        return "UUID(\(e.getUUIDValue()?.description ?? "?"))"
    case kBluetoothSDPDataElementTypeString:
        return "\"\(e.getStringValue() ?? "")\""
    case kBluetoothSDPDataElementTypeDataElementSequence, kBluetoothSDPDataElementTypeDataElementAlternative:
        let arr = (e.getArrayValue() as? [IOBluetoothSDPDataElement]) ?? []
        return "[" + arr.map(describe).joined(separator: ", ") + "]"
    case kBluetoothSDPDataElementTypeUnsignedInt, kBluetoothSDPDataElementTypeSignedInt:
        return e.getNumberValue()?.stringValue ?? "?"
    case kBluetoothSDPDataElementTypeBoolean:
        return e.getNumberValue()?.boolValue == true ? "true" : "false"
    default:
        return "type\(t)"
    }
}

print("\n--- cached services ---")
dumpServices(dev)

print("\n--- performing fresh SDP query (may page the device) ---")
let d = SDPDelegate()
let r = dev.performSDPQuery(d)
print("performSDPQuery returned \(String(format: "0x%08X", r))")
let deadline = Date().addingTimeInterval(30)
while !d.done && Date() < deadline {
    RunLoop.current.run(until: Date().addingTimeInterval(0.2))
}
if !d.done { print("SDP query timed out") }
dumpServices(dev)
print("\nisConnected after SDP: \(dev.isConnected())")
if dev.isConnected() { dev.closeConnection() }
