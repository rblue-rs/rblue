// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.panic;

/**
 * Exception that represents a Rust panic. This exception can be used to carry
 * Rust panics across the JNI boundary.
 * <p>
 * Instances of this class cannot be obtained directly from Java. Instead, call
 * {@code jni_utils::exceptions::JPanicException::new())} to create a new
 * instance of this class.
 */
public final class PanicException extends RuntimeException {
    @SuppressWarnings("unused") // Native code uses this field.
    private long any;

    private PanicException(String message) {
        super(message);
    }
}
