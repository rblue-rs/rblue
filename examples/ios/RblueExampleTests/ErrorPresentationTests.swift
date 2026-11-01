import XCTest

import Rblue
@testable import RblueExample

/// Structured error presentation: every RblueError case keeps its fields,
/// raw platform codes are shown verbatim.
final class ErrorPresentationTests: XCTestCase {
    func testInvalidLengthKeepsStructuredFields() {
        let presented = PresentedError.from(
            RblueError.InvalidLength(len: 513, max: 512)
        )
        XCTAssertTrue(presented.title.contains("InvalidLength"))
        XCTAssertTrue(presented.detail.contains("513"))
        XCTAssertTrue(presented.detail.contains("512"))
        XCTAssertTrue(presented.structured.contains("513"))
    }

    func testPlatformCodeShownVerbatim() {
        let presented = PresentedError.from(
            RblueError.Platform(code: 133, message: "simulated GATT failure")
        )
        XCTAssertTrue(presented.title.contains("平台错误"))
        XCTAssertTrue(presented.detail.contains("133"))
        XCTAssertTrue(presented.structured.contains("133"))
    }

    func testPermissionReasonsMapToActions() {
        let off = PresentedError.from(RblueError.Permission(reason: .bluetoothOff))
        XCTAssertEqual(off.title, "蓝牙已关闭")
        XCTAssertTrue(off.detail.contains("控制中心"))

        let denied = PresentedError.from(RblueError.Permission(reason: .unauthorized))
        XCTAssertEqual(denied.title, "蓝牙权限未授权")
        XCTAssertTrue(denied.detail.contains("隐私"))
    }

    func testUnsupportedIsPresentedAsPlatformFact() {
        let presented = PresentedError.from(
            RblueError.Unsupported(platform: "ios", feature: "mtu()")
        )
        XCTAssertEqual(presented.title, "不支持")
        XCTAssertTrue(presented.detail.contains("ios"))
        XCTAssertTrue(presented.detail.contains("mtu()"))
    }

    func testForeignErrorFallsBackToGenericEntry() {
        let presented = PresentedError.from(
            NSError(domain: "test-domain", code: 42)
        )
        XCTAssertEqual(presented.title, "错误")
        XCTAssertTrue(presented.detail.contains("42"))
    }
}
