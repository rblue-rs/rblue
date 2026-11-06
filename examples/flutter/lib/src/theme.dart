import 'package:flutter/material.dart';

/// Static M3 color roles and selective emphasized typography from the prototype.
ThemeData buildExampleTheme(Brightness brightness) {
  final dark = brightness == Brightness.dark;
  final base = ColorScheme.fromSeed(
    seedColor: const Color(0xff6555ce),
    brightness: brightness,
  );
  final colors = base.copyWith(
    tertiary: Color(dark ? 0xffb2d266 : 0xff4d6700),
    onTertiary: Color(dark ? 0xff263500 : 0xffffffff),
    tertiaryContainer: Color(dark ? 0xff394d00 : 0xffcdef7f),
    onTertiaryContainer: Color(dark ? 0xffcdef7f : 0xff151f00),
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
      fillColor: colors.surfaceContainerHighest,
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: BorderSide.none,
      ),
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: colors.surfaceContainer,
      indicatorColor: colors.secondaryContainer,
      labelTextStyle: WidgetStateProperty.resolveWith(
        (states) => TextStyle(
          fontSize: 12,
          color: states.contains(WidgetState.selected)
              ? colors.secondary
              : colors.onSurfaceVariant,
          fontWeight: states.contains(WidgetState.selected)
              ? FontWeight.w700
              : FontWeight.w500,
        ),
      ),
    ),
  );
}
