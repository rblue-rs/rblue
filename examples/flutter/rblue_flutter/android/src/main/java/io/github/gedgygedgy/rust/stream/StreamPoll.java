// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.stream;

/**
 * Represents the result of polling an async stream.
 * <p>
 * See {@link Stream#pollNext} for a description of how to use this interface.
 */
public interface StreamPoll<T> {
    /**
     * Gets the stream item. This can be anything, including {@code null}.
     *
     * @return The stream item.
     */
    T get();
}
