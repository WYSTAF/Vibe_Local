package com.vibelocal.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.vibelocal.app.ui.MainScreen
import com.vibelocal.app.ui.theme.VibeLocalTheme

class MainActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            VibeLocalTheme {
                MainScreen()
            }
        }
    }
}