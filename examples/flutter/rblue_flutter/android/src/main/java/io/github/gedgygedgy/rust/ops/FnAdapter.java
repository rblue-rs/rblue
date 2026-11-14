// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.ops;

import io.github.gedgygedgy.rust.thread.LocalThreadChecker;

import java.io.Closeable;

final class FnAdapter<O, T, U, R> implements Closeable {
    private final LocalThreadChecker threadChecker;
    private long data;

    private FnAdapter(boolean local) {
        this.threadChecker = new LocalThreadChecker(local);
    }

    public R call(O self, T arg1, U arg2) {
        this.threadChecker.check();
        return this.callInternal(self, arg1, arg2);
    }

    private native R callInternal(O self, T arg1, U arg2);

    @Override
    public void close() {
        this.threadChecker.check();
        this.closeInternal();
    }

    private native void closeInternal();
}
