// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.task;

/**
 * Represents the result of polling an async future.
 * <p>
 * If a future result is available, the future should return an instance of
 * this interface which returns the result from the {@link get} method.
 * If no result is available, the future should return {@code null} (NOT a
 * {@link PollResult} whose {@link get} returns {@code null}.) A
 * non-{@code null} {@link PollResult} indicates that the future has completed.
 */
public interface PollResult<T> {
    /**
     * Gets the future result. This can be anything, including {@code null}.
     *
     * @return The future result.
     */
    public T get();
}
