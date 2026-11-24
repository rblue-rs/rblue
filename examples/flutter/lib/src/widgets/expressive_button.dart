import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';

final _fastSpatial = SpringDescription.withDampingRatio(
  mass: 1,
  stiffness: 800,
  ratio: 0.6,
);

/// Native Material button with interruptible M3E pressed/selected shape morphing.
class ExpressiveButton extends StatefulWidget {
  const ExpressiveButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.icon,
    this.tonal = false,
    this.selected = false,
    this.onHighlightChanged,
    this.height = 56,
  });

  final String label;
  final VoidCallback? onPressed;
  final IconData? icon;
  final bool tonal;
  final bool selected;
  final ValueChanged<bool>? onHighlightChanged;
  final double height;

  @override
  State<ExpressiveButton> createState() => _ExpressiveButtonState();
}

class _ExpressiveButtonState extends State<ExpressiveButton>
    with SingleTickerProviderStateMixin {
  late final _animation = AnimationController.unbounded(vsync: this);
  final _states = WidgetStatesController();
  var _pressed = false;

  @override
  void initState() {
    super.initState();
    _states.addListener(_onStatesChanged);
  }

  void _onStatesChanged() {
    final pressed = _states.value.contains(WidgetState.pressed);
    if (pressed == _pressed) return;
    _pressed = pressed;
    widget.onHighlightChanged?.call(pressed);
    final target = pressed ? 1.0 : 0.0;
    if (MediaQuery.disableAnimationsOf(context)) {
      _animation.value = target;
    } else {
      _animation.animateWith(
        SpringSimulation(
          _fastSpatial,
          _animation.value,
          target,
          _animation.velocity,
        ),
      );
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _animation.stop();
      _animation.value = _pressed ? 1 : 0;
    }
  }

  @override
  void dispose() {
    _states.removeListener(_onStatesChanged);
    _states.dispose();
    _animation.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
    animation: _animation,
    builder: (context, _) {
      final resting = widget.selected ? 16.0 : widget.height / 2;
      final pressed = widget.height >= 96 ? 16.0 : 12.0;
      final radius = (resting + (pressed - resting) * _animation.value).clamp(
        0.0,
        widget.height / 2,
      );
      final style = FilledButton.styleFrom(
        minimumSize: Size(48, widget.height),
        padding: const EdgeInsets.symmetric(horizontal: 16),
        textStyle: const TextStyle(fontSize: 14, fontWeight: FontWeight.w700),
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(radius),
        ),
        // Geometry is driven by the spring, not ButtonStyle's implicit tween.
        animationDuration: Duration.zero,
      );
      final child = Row(
        mainAxisSize: MainAxisSize.min,
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          if (widget.icon != null) ...[
            Icon(widget.icon, size: 24),
            const SizedBox(width: 8),
          ],
          Flexible(child: Text(widget.label, maxLines: 1)),
        ],
      );
      return widget.tonal
          ? FilledButton.tonal(
              statesController: _states,
              style: style,
              onPressed: widget.onPressed,
              child: child,
            )
          : FilledButton(
              statesController: _states,
              style: style,
              onPressed: widget.onPressed,
              child: child,
            );
    },
  );
}

/// Related actions whose immediate neighbors respond to a pressed button.
class SquishButtonGroup extends StatefulWidget {
  const SquishButtonGroup({super.key, required this.actions});

  final List<({String label, VoidCallback? onPressed, bool tonal})> actions;

  @override
  State<SquishButtonGroup> createState() => _SquishButtonGroupState();
}

class _SquishButtonGroupState extends State<SquishButtonGroup>
    with SingleTickerProviderStateMixin {
  late final _animation = AnimationController.unbounded(vsync: this);
  int? _pressed;

  void _highlight(int index, bool pressed) {
    if (!mounted) return;
    if (pressed) _pressed = index;
    final target = pressed ? 1.0 : 0.0;
    if (MediaQuery.disableAnimationsOf(context)) {
      _animation.value = target;
    } else {
      _animation.animateWith(
        SpringSimulation(
          _fastSpatial,
          _animation.value,
          target,
          _animation.velocity,
        ),
      );
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _animation.stop();
      _animation.value = 0;
    }
  }

  @override
  void dispose() {
    _animation.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final largeText = MediaQuery.textScalerOf(context).scale(14) > 20;
    return LayoutBuilder(
      builder: (context, constraints) {
        final tight =
            constraints.maxWidth.isFinite &&
            constraints.maxWidth < 88 * widget.actions.length;
        if (largeText || tight) {
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final action in widget.actions) ...[
                ExpressiveButton(
                  label: action.label,
                  onPressed: action.onPressed,
                  tonal: action.tonal,
                ),
                const SizedBox(height: 8),
              ],
            ],
          );
        }
        return _row();
      },
    );
  }

  Widget _row() {
    return AnimatedBuilder(
      animation: _animation,
      builder: (context, _) => Row(
        children: [
          for (var index = 0; index < widget.actions.length; index++) ...[
            if (index > 0) const SizedBox(width: 8),
            Expanded(
              flex:
                  (1000 *
                          (1 +
                              (_pressed == index
                                      ? .18
                                      : _pressed != null &&
                                            (_pressed! - index).abs() == 1
                                      ? -.18
                                      : 0) *
                                  _animation.value))
                      .round()
                      .clamp(600, 1400),
              child: ExpressiveButton(
                label: widget.actions[index].label,
                tonal: widget.actions[index].tonal,
                onPressed: widget.actions[index].onPressed,
                onHighlightChanged: (pressed) => _highlight(index, pressed),
              ),
            ),
          ],
        ],
      ),
    );
  }
}
