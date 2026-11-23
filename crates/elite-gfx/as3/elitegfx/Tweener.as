package elitegfx {
    import flash.display.Shape;
    import flash.events.Event;
    import flash.external.ExternalInterface;

    // The tweener Elite's exe gives the movies (TweenerFactory.getTweenerObject): a STAND-IN (grade B,
    // ARCHITECTURE.md A10; the exe's code was not read). The movies call
    //   addTween(target, seconds, {property: end, ...}, easing, delay, onComplete)
    //   removeAllTweens()
    // (every call site has these shapes: 4 to 6 arguments). Rules:
    // - one clock for all tweeners, stepped one frame of the movie (1 / frame rate) on each frame;
    // - a tween starts when its delay has passed; each property then runs from the value it has at
    //   that moment to its end value, along Elite's numbered curve (src/easing.rs, asked by "Ease");
    // - at its end every property is exactly its end value, then onComplete is called;
    // - a new tween takes over the properties it names on that target from older tweens (of any
    //   tweener); an older tween left with nothing to move ends there, without its onComplete;
    // - removeAllTweens drops this tweener's tweens, without their onComplete.
    public class Tweener {
        private static var clock:Shape;
        private static var frame:Number = 1 / 30;
        private static var now:Number = 0;
        // Every running tween, oldest first: {owner, target, seconds, ends, easing, start, from, done}.
        private static var live:Array = [];

        // Starts the clock at the movie's frame rate (the bridge's bind).
        public static function start(frameRate:Number):void {
            if (frameRate > 0) {
                frame = 1 / frameRate;
            }
            if (clock == null) {
                clock = new Shape();
                clock.addEventListener(Event.ENTER_FRAME, tick);
            }
        }

        public function addTween(target:Object, seconds:Number, ends:Object, easing:int = 0, delay:Number = 0,
                                 onComplete:Function = null, ...rest):void {
            var names:Array = [];
            for (var name:String in ends) names.push(name);
            takeOver(target, names);
            live.push({owner: this, target: target, seconds: seconds, ends: ends, names: names, easing: easing,
                       start: now + Math.max(0, delay), from: null, done: onComplete});
        }

        public function removeAllTweens(...rest):void {
            var kept:Array = [];
            for each (var t:Object in live) {
                if (t.owner !== this) kept.push(t);
            }
            live = kept;
        }

        private static function takeOver(target:Object, names:Array):void {
            if (names.length == 0) return;
            var kept:Array = [];
            for each (var t:Object in live) {
                if (t.target === target) {
                    t.names = t.names.filter(function(n:String, i:int, a:Array):Boolean {
                        return names.indexOf(n) < 0;
                    });
                    if (t.names.length == 0 && t.ends != null && hasAny(t.ends)) continue;
                }
                kept.push(t);
            }
            live = kept;
        }

        // Moves a started tween's properties to this frame's values; whether it has ended.
        private static function step(t:Object):Boolean {
            var name:String;
            if (t.from == null) {
                var from:Object = {};
                for each (name in t.names) from[name] = Number(t.target[name]);
                t.from = from;
            }
            var p:Number = t.seconds <= 0 ? 1 : Math.min(1, (now - t.start) / t.seconds);
            var k:Number = p >= 1 ? 1 : (t.easing == 0 ? p : Number(ExternalInterface.call("Ease", t.easing, p)));
            for each (name in t.names) {
                var end:Number = Number(t.ends[name]);
                t.target[name] = p >= 1 ? end : t.from[name] + (end - t.from[name]) * k;
            }
            return p >= 1;
        }

        private static function hasAny(o:Object):Boolean {
            for (var n:String in o) return true;
            return false;
        }

        private static function tick(e:Event):void {
            now += frame;
            var finished:Array = [];
            var broken:Array = [];
            for each (var t:Object in live.concat()) {
                if (now < t.start - 1e-9) continue;
                // A target that cannot be read or written (null, a throwing setter) drops only its
                // own tween, without its onComplete; the others go on.
                try {
                    if (step(t)) finished.push(t);
                } catch (error:Error) {
                    broken.push(t);
                    ExternalInterface.call("Trace", "tween dropped: " + (error.getStackTrace() || error.toString()));
                }
            }
            if (finished.length == 0 && broken.length == 0) return;
            var kept:Array = [];
            for each (var u:Object in live) {
                if (finished.indexOf(u) < 0 && broken.indexOf(u) < 0) kept.push(u);
            }
            live = kept;
            for each (var f:Object in finished) {
                if (f.done == null) continue;
                try {
                    f.done();
                } catch (error:Error) {
                    ExternalInterface.call("Trace", "tween onComplete: " + (error.getStackTrace() || error.toString()));
                }
            }
        }
    }
}
