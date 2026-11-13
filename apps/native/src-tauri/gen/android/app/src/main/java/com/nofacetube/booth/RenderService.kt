package com.nofacetube.booth

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * Runs while Booth loads the voice engine or renders a take, with a quiet notification. A
 * foreground service tells Android the work is wanted, so the system doesn't freeze Booth or
 * kill the engine (a child process) for using the processor while you're in another app.
 * Started and stopped from Rust (booth_lib::android_work) through [set].
 */
class RenderService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val nm = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= 26) {
      nm.createNotificationChannel(
        NotificationChannel(CHANNEL, "Generating speech", NotificationManager.IMPORTANCE_LOW).apply {
          description = "Shown while Booth renders a take, so it keeps going in the background."
          setShowBadge(false)
        }
      )
    }
    val open = PendingIntent.getActivity(
      this, 0,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_IMMUTABLE
    )
    val note = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(R.drawable.ic_stat_booth)
      .setContentTitle("Generating speech")
      .setContentText("Booth keeps going while you use other apps.")
      .setContentIntent(open)
      .setOngoing(true)
      .setSilent(true)
      .setCategory(NotificationCompat.CATEGORY_PROGRESS)
      .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
      .build()
    // Media processing is the type made for this, but some Android builds turn it down (Android 17
    // on a Galaxy S25 did); data sync is the fallback. Before Android 10 there are no types.
    val types = buildList {
      if (Build.VERSION.SDK_INT >= 35) add(ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING)
      if (Build.VERSION.SDK_INT >= 29) add(ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
      if (isEmpty()) add(0)
    }
    val ok = types.any { type ->
      try {
        ServiceCompat.startForeground(this, NOTE_ID, note, type)
        Log.i(TAG, "in the foreground as type $type")
        true
      } catch (e: Exception) {
        Log.w(TAG, "foreground type $type refused: ${e.message}")
        false
      }
    }
    if (!ok) {
      running = false
      stopSelf()
    }
    return START_NOT_STICKY
  }

  // Android 15 caps media processing at 6 hours a day; past that, stop instead of being killed.
  override fun onTimeout(startId: Int, fgsType: Int) {
    running = false
    stopSelf()
  }

  override fun onDestroy() {
    running = false
    super.onDestroy()
  }

  companion object {
    private const val TAG = "BoothRender"
    private const val CHANNEL = "render"
    private const val NOTE_ID = 7
    @Volatile private var running = false

    /** On while the engine is loading or rendering; off when it's idle. Any thread. */
    @JvmStatic
    fun set(ctx: Context, on: Boolean) {
      Handler(Looper.getMainLooper()).post {
        val intent = Intent(ctx, RenderService::class.java)
        if (on && !running) {
          MainActivity.askForNotifications()
          try {
            ContextCompat.startForegroundService(ctx, intent)
            running = true
          } catch (e: Exception) {
            // Android 12+ refuses to start one from the background; the take still runs.
            Log.w(TAG, "couldn't start the render service", e)
          }
        } else if (!on && running) {
          ctx.stopService(intent)
          running = false
        }
      }
    }
  }
}
