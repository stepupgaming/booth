# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
# Rust calls these by name over JNI (see android_work in src/lib.rs), so R8 must keep them.
-keep class com.nofacetube.booth.RenderService { public static void set(android.content.Context, boolean); }
-keep class com.nofacetube.booth.RenderService$Companion { *; }
-keep class com.nofacetube.booth.MainActivity$Companion { *; }
