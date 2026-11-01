import XCTest

/// JSON helpers shared by the contract tests.
enum TestJSON {
    /// Parses a facade JSON string into an object for comparison. Key order
    /// is an encoder artifact (serde accepts any order), so contract tests
    /// compare parsed objects, never raw strings.
    static func object(_ json: String) throws -> NSDictionary {
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
        return try XCTUnwrap(object as? NSDictionary, "not a JSON object: \(json)")
    }
}
