# The Rust core finds these by name over JNI.
-keep class dev.wiltaylor.thevault.device.VaultCore { native <methods>; }
-keep interface dev.wiltaylor.thevault.device.DeviceBridge { *; }
-keep class * implements dev.wiltaylor.thevault.device.DeviceBridge { public java.lang.String call(java.lang.String, java.lang.String); }
-keep interface dev.wiltaylor.thevault.device.ImportProgress { *; }
-keep class * implements dev.wiltaylor.thevault.device.ImportProgress { public void update(java.lang.String); }
