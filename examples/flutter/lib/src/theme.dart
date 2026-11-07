import 'package:flutter/material.dart';

/// M3 color roles and emphasized typography for the BLE workbench.
ThemeData buildExampleTheme(Brightness brightness) {
  final base = ColorScheme.fromSeed(
    seedColor: const Color(0xff2469c7),
    brightness: brightness,
  );
  final colors = brightness == Brightness.dark
      ? base.copyWith(
          surface: const Color(0xff000000),
          surfaceDim: const Color(0xff000000),
          surfaceBright: const Color(0xff242426),
          surfaceContainerLowest: const Color(0xff0a0a0b),
          surfaceContainerLow: const Color(0xff121214),
          surfaceContainer: const Color(0xff18181b),
          surfaceContainerHigh: const Color(0xff222226),
          surfaceContainerHighest: const Color(0xff2a2a30),
          onSurface: const Color(0xfff0eff1),
          onSurfaceVariant: const Color(0xffc4c3c7),
          outlineVariant: const Color(0xff3e3f45),
          primaryContainer: const Color(0xff171b22),
          onPrimaryContainer: const Color(0xfff0eff1),
          secondaryContainer: const Color(0xff18202a),
          onSecondaryContainer: const Color(0xfff0eff1),
        )
      : base.copyWith(
          surface: const Color(0xfff3f5f7),
          surfaceDim: const Color(0xffd8dde4),
          surfaceBright: const Color(0xfff8f9fa),
          surfaceContainerLowest: const Color(0xffffffff),
          surfaceContainerLow: const Color(0xffffffff),
          surfaceContainer: const Color(0xffeef1f4),
          surfaceContainerHigh: const Color(0xffe5e9ee),
          surfaceContainerHighest: const Color(0xffdce2e8),
          onSurface: const Color(0xff181c22),
          onSurfaceVariant: const Color(0xff505a68),
          outline: const Color(0xff737e8d),
          outlineVariant: const Color(0xffc9d0d9),
          primary: const Color(0xff1f5fae),
          onPrimary: const Color(0xffffffff),
          primaryContainer: const Color(0xffe8eef6),
          onPrimaryContainer: const Color(0xff181c22),
          secondaryContainer: const Color(0xffe5edf7),
          onSecondaryContainer: const Color(0xff181c22),
        );
  final theme = ThemeData(useMaterial3: true, colorScheme: colors);
  return theme.copyWith(
    scaffoldBackgroundColor: colors.surface,
    textTheme: theme.textTheme.copyWith(
      displayLarge: theme.textTheme.displayLarge?.copyWith(
        fontWeight: FontWeight.w500,
        letterSpacing: 0,
      ),
      headlineLarge: theme.textTheme.headlineLarge?.copyWith(
        fontWeight: FontWeight.w500,
        letterSpacing: 0,
      ),
      titleMedium: theme.textTheme.titleMedium?.copyWith(
        fontWeight: FontWeight.w700,
        letterSpacing: 0.15,
      ),
    ),
    appBarTheme: AppBarTheme(
      centerTitle: false,
      backgroundColor: colors.surface,
      surfaceTintColor: Colors.transparent,
      scrolledUnderElevation: 0,
    ),
    cardTheme: CardThemeData(
      color: colors.surfaceContainerLow,
      elevation: 0,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(20)),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: colors.surfaceContainerHigh,
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: BorderSide.none,
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: BorderSide.none,
      ),
    ),
  );
}
