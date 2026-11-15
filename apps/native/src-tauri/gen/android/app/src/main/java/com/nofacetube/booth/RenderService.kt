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
import android.os.PowerManager
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
    val note = build(this)
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
    } else if (wake == null) {
      // A foreground service alone doesn't keep the processor awake with the screen off; a
      // partial wake lock does (the screen still turns off). Capped so it can't outlive a bug.
      wake = getSystemService(PowerManager::class.java)
        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "Booth:render")
        .apply { setReferenceCounted(false); acquire(3 * 60 * 60 * 1000L) }
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
    wake?.let { if (it.isHeld) it.release() }
    wake = null
    super.onDestroy()
  }

  private var wake: PowerManager.WakeLock? = null

  companion object {
    private const val TAG = "BoothRender"
    private const val CHANNEL = "render"
    private const val NOTE_ID = 7
    @Volatile private var running = false
    @Volatile private var title = "Generating speech"
    @Volatile private var text = "Booth keeps going while you use other apps."

    /** What the notification says, e.g. "Making line 4 of 7" while a script renders. Any thread. */
    @JvmStatic
    fun status(ctx: Context, newTitle: String, newText: String) {
      title = newTitle
      text = newText
      // Posting with the service's own id replaces the notification it shows.
      if (running) {
        try {
          ctx.getSystemService(NotificationManager::class.java).notify(NOTE_ID, build(ctx))
        } catch (e: Exception) {
          Log.w(TAG, "couldn't update the notification", e)
        }
      }
    }

    private fun build(ctx: Context): android.app.Notification {
      val open = PendingIntent.getActivity(
        ctx, 0,
        Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE
      )
      return NotificationCompat.Builder(ctx, CHANNEL)
        .setSmallIcon(R.drawable.ic_stat_booth)
        .setContentTitle(title)
        .setContentText(text)
        .setContentIntent(open)
        .setOngoing(true)
        .setSilent(true)
        .setOnlyAlertOnce(true)
        .setCategory(NotificationCompat.CATEGORY_PROGRESS)
        .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        .build()
    }

    /** Back to the plain wording once a script is done. */
    @JvmStatic
    fun plain() {
      title = "Generating speech"
      text = "Booth keeps going while you use other apps."
    }

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
