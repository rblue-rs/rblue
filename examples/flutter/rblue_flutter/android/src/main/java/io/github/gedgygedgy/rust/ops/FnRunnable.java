// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.ops;

import java.io.Closeable;

/**
 * Wraps a closure in a Java object.
 * <p>
 * Instances of this class cannot be obtained directly from Java. Instead, call
 * {@code jni_utils::ops::fn_runnable()},
 * {@code jni_utils::ops::fn_mut_runnable()}, or
 * {@code jni_utils::ops::fn_once_runnable()} from Rust code to obtain an
 * instance of this class.
 */
public interface FnRunnable extends Runnable, Closeable {
    /**
     * Runs the closure associated with this object.
     * <p>
     * If the closure is a {@code std::ops::Fn} or {@code std::ops::FnMut},
     * calling this method twice will call the associated closure twice. If
     * the closure is a {@code std::ops::FnOnce}, this method is idempotent -
     * calling it a second time will have no effect. If {@link close} has
     * already been called, this method is a no-op.
     */
    @Override
    public void run();

    /**
     * Disposes of the closure associated with this object.
     * <p>
     * This method is idempotent - if it's called twice, the second call is a
     * no-op.
     */
    @Override
    public void close();
}
