// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.thread;

public class LocalThreadChecker {
    private final Thread origin;

    public LocalThreadChecker(boolean local) {
        this.origin = local ? Thread.currentThread() : null;
    }

    public void check() {
        if (this.origin != null && this.origin != Thread.currentThread()) {
            throw new LocalThreadException();
        }
    }
}
