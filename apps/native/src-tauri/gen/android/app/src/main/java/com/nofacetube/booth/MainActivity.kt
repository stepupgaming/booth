package com.nofacetube.booth

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import java.lang.ref.WeakReference

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    current = WeakReference(this)
    // Android 15+ always draws apps behind the status and navigation bars. Keep Booth's page
    // between them, and above the keyboard while typing a line.
    val content = findViewById<View>(android.R.id.content)
    content.setBackgroundColor(0xFF07080D.toInt())
    ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      )
      v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }

  companion object {
    private var current: WeakReference<MainActivity>? = null

    /**
     * Android 13+ hides the "Generating speech" notification unless the app may post
     * notifications. Ask once, the first time Booth renders; the render runs either way.
     */
    fun askForNotifications() {
      val a = current?.get() ?: return
      if (Build.VERSION.SDK_INT < 33) return
      if (a.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
      val prefs = a.getSharedPreferences("booth", MODE_PRIVATE)
      if (prefs.getBoolean("asked_notifications", false)) return
      prefs.edit().putBoolean("asked_notifications", true).apply()
      a.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 7)
    }
  }
}
