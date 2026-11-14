// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.future;

/**
 * Exception class for {@link Future} implementations to throw from
 * {@link io.github.gedgygedgy.rust.task.PollResult#get} if the result of the
 * future is an exception. Implementations should set the real exception as the
 * cause of this exception.
 */
public class FutureException extends RuntimeException {
    public FutureException(Throwable cause) {
        super(cause);
    }
}
