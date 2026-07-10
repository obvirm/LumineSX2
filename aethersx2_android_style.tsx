import React, { useState, useEffect, useRef } from 'react';
import { 
  Menu, 
  Search, 
  Grid, 
  List, 
  MoreVertical,
  Play,
  Settings,
  FolderPlus,
  Gamepad2,
  Maximize,
  ArrowLeft,
  ChevronRight,
  MonitorSmartphone,
  Save,
  Cpu,
  Volume2,
  Trash2,
  Check,
  Plus,
  Folder,
  RotateCcw,
  Sliders,
  Info
} from 'lucide-react';

const initialMockGames = [
  { id: 1, title: "Grand Theft Auto: San Andreas", region: "NTSC-U", size: "4.2 GB", code: "SLUS-20946", color: "from-orange-600 to-yellow-600", favorite: true },
  { id: 2, title: "God of War II", region: "NTSC-U", size: "8.5 GB", code: "SCUS-97481", color: "from-red-700 to-red-900", favorite: true },
  { id: 3, title: "Resident Evil 4", region: "PAL", size: "3.1 GB", code: "SLES-53464", color: "from-slate-700 to-slate-900", favorite: false },
  { id: 4, title: "Final Fantasy X", region: "NTSC-J", size: "4.0 GB", code: "SLPS-25088", color: "from-blue-600 to-cyan-800", favorite: false },
  { id: 5, title: "Need for Speed: Most Wanted", region: "NTSC-U", size: "2.5 GB", code: "SLUS-21165", color: "from-gray-600 to-gray-800", favorite: false },
  { id: 6, title: "Bully", region: "NTSC-U", size: "2.8 GB", code: "SLUS-21269", color: "from-yellow-600 to-amber-800", favorite: false },
];

const mockBiosFiles = [
  { id: 'scph39001', name: "SCPH-39001 (USA v01.60)", selected: true, status: "Valid" },
  { id: 'scph70008', name: "SCPH-70008 (Europe v02.00)", selected: false, status: "Valid" },
  { id: 'scph10000', name: "SCPH-10000 (Japan v01.00)", selected: false, status: "Valid" },
];

export default function App() {
  const [viewMode, setViewMode] = useState('grid');
  const [activeTab, setActiveTab] = useState('games');
  const [isPlaying, setIsPlaying] = useState(false);
  const [selectedGame, setSelectedGame] = useState(null);
  
  // New Enhanced States
  const [themeColor, setThemeColor] = useState('purple'); // purple, emerald, amber, blue, rose
  const [searchQuery, setSearchQuery] = useState('');
  const [regionFilter, setRegionFilter] = useState('All'); // All, NTSC-U, PAL, NTSC-J
  const [gamesList, setGamesList] = useState(initialMockGames);
  const [biosList, setBiosList] = useState(mockBiosFiles);
  const [showImportModal, setShowImportModal] = useState(false);
  const [toastMessage, setToastMessage] = useState(null);
  
  // Game importing form states
  const [newGameTitle, setNewGameTitle] = useState('');
  const [newGameRegion, setNewGameRegion] = useState('NTSC-U');
  const [newGameSize, setNewGameSize] = useState('1.5 GB');
  const [newGameColor, setNewGameColor] = useState('from-indigo-600 to-purple-600');

  // In-Game State Overlays
  const [showQuickMenu, setShowQuickMenu] = useState(false);
  const [gameSpeed, setGameSpeed] = useState(100); // speed percentage
  const [currentFps, setCurrentFps] = useState(60);

  // Simulated Save States List
  const [saveStates, setSaveStates] = useState([
    { id: 101, gameTitle: "God of War II", timestamp: "Today, 14:23", screenshotColor: "from-red-800 to-zinc-900", progress: "Boss Battle - Colossus" },
    { id: 102, gameTitle: "GTA: San Andreas", timestamp: "Yesterday, 18:45", screenshotColor: "from-orange-800 to-zinc-900", progress: "Grove Street Safehouse" }
  ]);

  // Canvas ref for gameplay simulation
  const canvasRef = useRef(null);

  // Material You Theme palette classes configuration
  const themes = {
    purple: {
      primary: '#D0BCFF',
      onPrimary: '#381E72',
      primaryContainer: '#4A4458',
      textPrimary: 'text-[#D0BCFF]',
      bgPrimary: 'bg-[#D0BCFF]',
      borderPrimary: 'border-[#D0BCFF]',
      accentBg: 'bg-[#D0BCFF]',
      accentText: 'text-[#E8DEF8]'
    },
    emerald: {
      primary: '#A7F3D0',
      onPrimary: '#064E3B',
      primaryContainer: '#065F46',
      textPrimary: 'text-[#34D399]',
      bgPrimary: 'bg-[#34D399]',
      borderPrimary: 'border-[#34D399]',
      accentBg: 'bg-[#34D399]',
      accentText: 'text-[#A7F3D0]'
    },
    amber: {
      primary: '#FDE68A',
      onPrimary: '#78350F',
      primaryContainer: '#451A03',
      textPrimary: 'text-[#FBBF24]',
      bgPrimary: 'bg-[#FBBF24]',
      borderPrimary: 'border-[#FBBF24]',
      accentBg: 'bg-[#FBBF24]',
      accentText: 'text-[#FDE68A]'
    },
    blue: {
      primary: '#93C5FD',
      onPrimary: '#1E3A8A',
      primaryContainer: '#1E40AF',
      textPrimary: 'text-[#60A5FA]',
      bgPrimary: 'bg-[#60A5FA]',
      borderPrimary: 'border-[#60A5FA]',
      accentBg: 'bg-[#60A5FA]',
      accentText: 'text-[#93C5FD]'
    },
    rose: {
      primary: '#FECDD3',
      onPrimary: '#881337',
      primaryContainer: '#4C0519',
      textPrimary: 'text-[#FB7185]',
      bgPrimary: 'bg-[#FB7185]',
      borderPrimary: 'border-[#FB7185]',
      accentBg: 'bg-[#FB7185]',
      accentText: 'text-[#FECDD3]'
    }
  };

  const activeTheme = themes[themeColor];

  // Show visual in-app notifications
  const triggerToast = (msg) => {
    setToastMessage(msg);
    setTimeout(() => {
      setToastMessage(null);
    }, 3000);
  };

  // Toggle favorite game
  const toggleFavorite = (id, e) => {
    e.stopPropagation();
    setGamesList(prev => prev.map(g => g.id === id ? { ...g, favorite: !g.favorite } : g));
    const game = gamesList.find(g => g.id === id);
    triggerToast(game?.favorite ? "Removed from Favorites" : "Added to Favorites");
  };

  // Handle ROM mock importing
  const handleImportGame = (e) => {
    e.preventDefault();
    if (!newGameTitle.trim()) return;

    const newGame = {
      id: Date.now(),
      title: newGameTitle,
      region: newGameRegion,
      size: newGameSize,
      code: "SLUS-" + Math.floor(10000 + Math.random() * 90000),
      color: newGameColor,
      favorite: false
    };

    setGamesList(prev => [newGame, ...prev]);
    setShowImportModal(false);
    setNewGameTitle('');
    triggerToast(`Imported ${newGame.title} successfully!`);
  };

  // Handle BIOS Selection
  const selectBios = (id) => {
    setBiosList(prev => prev.map(b => b.id === id ? { ...b, selected: true } : { ...b, selected: false }));
    const selected = biosList.find(b => b.id === id);
    triggerToast(`Selected BIOS: ${selected?.name}`);
  };

  // Add Save State dynamically
  const createSaveState = () => {
    const newSlot = {
      id: Date.now(),
      gameTitle: selectedGame?.title || "Unknown Game",
      timestamp: "Just Now",
      screenshotColor: selectedGame?.color || "from-slate-700 to-slate-900",
      progress: "Auto Saved State Slot " + (saveStates.length + 1)
    };
    setSaveStates(prev => [newSlot, ...prev]);
    triggerToast("State Saved Successfully!");
  };

  // Delete Save State
  const deleteSaveState = (id, e) => {
    e.stopPropagation();
    setSaveStates(prev => prev.filter(item => item.id !== id));
    triggerToast("Save State deleted");
  };

  const startGame = (game) => {
    setSelectedGame(game);
    setIsPlaying(true);
    setShowQuickMenu(false);
    triggerToast(`Starting ${game.title}...`);
  };

  const stopGame = () => {
    setIsPlaying(false);
    setSelectedGame(null);
  };

  // Handle key listeners / D-pad simulations
  const [controllerPressed, setControllerPressed] = useState({
    up: false, down: false, left: false, right: false,
    cross: false, circle: false, square: false, triangle: false
  });

  const handleButtonPress = (btn, isPressed) => {
    setControllerPressed(prev => ({ ...prev, [btn]: isPressed }));
  };

  // Auto Gameplay simulator inside Canvas
  useEffect(() => {
    if (!isPlaying) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    let animationId;
    
    // Simple 2D game state (a bouncing spaceship / stars)
    let playerX = canvas.width / 2;
    let playerY = canvas.height - 30;
    let stars = Array.from({ length: 40 }, () => ({
      x: Math.random() * canvas.width,
      y: Math.random() * canvas.height,
      speed: Math.random() * 2 + 1
    }));
    let bullets = [];
    let enemyShip = { x: canvas.width / 2, y: 40, dir: 1, size: 20 };

    const render = () => {
      // Dynamic FPS simulator based on speed
      const targetFps = Math.floor(58 + Math.random() * 3) * (gameSpeed / 100);
      setCurrentFps(targetFps.toFixed(1));

      // Clear Screen
      ctx.fillStyle = '#050510';
      ctx.fillRect(0, 0, canvas.width, canvas.height);

      // Draw background nebula
      const gradient = ctx.createRadialGradient(canvas.width / 2, canvas.height / 2, 10, canvas.width / 2, canvas.height / 2, canvas.width);
      gradient.addColorStop(0, '#131124');
      gradient.addColorStop(1, '#050510');
      ctx.fillStyle = gradient;
      ctx.fillRect(0, 0, canvas.width, canvas.height);

      // Draw Stars
      ctx.fillStyle = '#ffffff';
      stars.forEach(star => {
        ctx.fillRect(star.x, star.y, 1.5, 1.5);
        star.y += star.speed * (gameSpeed / 100);
        if (star.y > canvas.height) {
          star.y = 0;
          star.x = Math.random() * canvas.width;
        }
      });

      // Handle controller logic on player location
      if (controllerPressed.left && playerX > 15) playerX -= 3;
      if (controllerPressed.right && playerX < canvas.width - 15) playerX += 3;
      if (controllerPressed.up && playerY > 50) playerY -= 3;
      if (controllerPressed.down && playerY < canvas.height - 15) playerY += 3;

      // Auto Fire bullet when tapping cross or circle
      if (controllerPressed.cross && bullets.length < 5) {
        bullets.push({ x: playerX, y: playerY - 10 });
        controllerPressed.cross = false; // shoot once per tap/hold simulation
      }

      // Draw Enemy
      ctx.fillStyle = '#f43f5e';
      ctx.beginPath();
      ctx.moveTo(enemyShip.x, enemyShip.y + 15);
      ctx.lineTo(enemyShip.x - 15, enemyShip.y - 10);
      ctx.lineTo(enemyShip.x + 15, enemyShip.y - 10);
      ctx.closePath();
      ctx.fill();

      // Enemy logic
      enemyShip.x += enemyShip.dir * 2 * (gameSpeed / 100);
      if (enemyShip.x > canvas.width - 25 || enemyShip.x < 25) {
        enemyShip.dir *= -1;
      }

      // Update Bullets
      ctx.fillStyle = '#60a5fa';
      bullets.forEach((bullet, index) => {
        ctx.fillRect(bullet.x - 2, bullet.y, 4, 10);
        bullet.y -= 5 * (gameSpeed / 100);
        
        // Bullet Hit collision check
        const dist = Math.hypot(bullet.x - enemyShip.x, bullet.y - enemyShip.y);
        if (dist < 20) {
          bullets.splice(index, 1);
          // Explode enemy briefly by changing coordinates
          enemyShip.x = Math.random() * (canvas.width - 50) + 25;
        }

        if (bullet.y < 0) bullets.splice(index, 1);
      });

      // Draw Player Spaceship
      ctx.fillStyle = '#a7f3d0';
      ctx.beginPath();
      ctx.moveTo(playerX, playerY - 12);
      ctx.lineTo(playerX - 12, playerY + 12);
      ctx.lineTo(playerX + 12, playerY + 12);
      ctx.closePath();
      ctx.fill();

      // UI HUD Overlay
      ctx.fillStyle = '#ffffff';
      ctx.font = '10px Courier New';
      ctx.fillText(`SPEED: ${gameSpeed}%`, 10, 20);
      ctx.fillText(`EMU: PS2 VIRTUAL ENGINE`, 10, 32);

      animationId = requestAnimationFrame(render);
    };

    render();
    return () => cancelAnimationFrame(animationId);
  }, [isPlaying, controllerPressed, gameSpeed]);

  // Filtering Logic for list
  const filteredGames = gamesList.filter(game => {
    const matchesSearch = game.title.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesRegion = regionFilter === 'All' || game.region === regionFilter;
    return matchesSearch && matchesRegion;
  });

  // Render Game view
  if (isPlaying) {
    return (
      <div className="fixed inset-0 bg-black flex items-center justify-center overflow-hidden touch-none selection:bg-transparent z-50">
        
        {/* Render interactive 2D gameplay simulator */}
        <div className="relative w-full h-full max-w-[850px] aspect-[4/3] bg-black border-x border-white/5 flex items-center justify-center">
          <canvas 
            ref={canvasRef} 
            width={640} 
            height={448} 
            className="w-full h-full max-h-screen object-contain"
          />
          
          {/* Quick Stats Overlay */}
          <div className="absolute top-2 left-1/2 -translate-x-1/2 flex space-x-3 bg-black/60 px-3 py-1 rounded-full border border-white/10 text-[11px] font-mono text-white/80 select-none">
            <span>{selectedGame?.title}</span>
            <span className="text-emerald-400">|</span>
            <span>{currentFps} FPS</span>
            <span className="text-emerald-400">|</span>
            <span>Speed: {gameSpeed}%</span>
          </div>
        </div>

        {/* Top Overlay Bar */}
        <div className="absolute top-0 left-0 right-0 p-4 flex justify-between items-start z-50 bg-gradient-to-b from-black/80 to-transparent">
          <button onClick={stopGame} className="bg-black/60 text-white p-3 rounded-full backdrop-blur-md border border-white/10 active:bg-white/20 transition-all">
            <ArrowLeft size={24} />
          </button>
          
          <div className="flex space-x-2">
            <button 
              onClick={() => setShowQuickMenu(!showQuickMenu)} 
              className={`p-3 rounded-full backdrop-blur-md border border-white/10 active:bg-white/20 transition-all text-white ${showQuickMenu ? 'bg-indigo-600' : 'bg-black/60'}`}
            >
              <Sliders size={20} />
            </button>
            <button onClick={createSaveState} className="bg-black/60 text-white p-3 rounded-full backdrop-blur-md border border-white/10 active:bg-white/20 transition-all">
              <Save size={20} />
            </button>
          </div>
        </div>

        {/* Floating Quick Action Overlay Menu (Drawer Style) */}
        {showQuickMenu && (
          <div className="absolute top-20 right-4 w-72 bg-[#1c1b22] border border-white/10 rounded-2xl p-4 shadow-2xl z-50 text-white">
            <div className="flex justify-between items-center mb-3">
              <h4 className="font-semibold text-sm tracking-wide">QUICK CONTROL PANEL</h4>
              <button onClick={() => setShowQuickMenu(false)} className="text-white/40 hover:text-white">✕</button>
            </div>
            
            <div className="space-y-4 text-xs">
              <div>
                <span className="text-white/60 block mb-1">EE CPU Cycle Rate (Speed):</span>
                <div className="flex items-center space-x-2">
                  <input 
                    type="range" 
                    min="50" 
                    max="150" 
                    value={gameSpeed} 
                    onChange={(e) => setGameSpeed(Number(e.target.value))}
                    className="w-full accent-[#D0BCFF] bg-white/10 rounded-lg appearance-none h-1"
                  />
                  <span className="font-mono">{gameSpeed}%</span>
                </div>
              </div>

              <div className="border-t border-white/10 pt-3">
                <button 
                  onClick={createSaveState}
                  className="w-full bg-[#36343b] active:bg-[#4a4850] py-2 rounded-lg font-medium flex items-center justify-center space-x-2 transition-colors mb-2"
                >
                  <Save size={16} />
                  <span>Instant Savestate</span>
                </button>
                <button 
                  onClick={() => { setGameSpeed(100); triggerToast("Engine Speed Reset"); }}
                  className="w-full bg-transparent border border-white/10 active:bg-white/5 py-2 rounded-lg font-medium flex items-center justify-center space-x-2 transition-colors"
                >
                  <RotateCcw size={16} />
                  <span>Reset Clock (100%)</span>
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Virtual Controller - Left (D-Pad) */}
        <div className="absolute bottom-12 left-8 w-44 h-44 z-40 opacity-40 hover:opacity-80 transition-opacity">
          <div className="relative w-full h-full mx-auto">
            {/* D-Pad Buttons */}
            <button 
              onMouseDown={() => handleButtonPress('up', true)}
              onMouseUp={() => handleButtonPress('up', false)}
              onTouchStart={() => handleButtonPress('up', true)}
              onTouchEnd={() => handleButtonPress('up', false)}
              className={`absolute top-0 left-1/2 -translate-x-1/2 w-14 h-14 bg-white/10 rounded-t-2xl border border-white/20 backdrop-blur-sm flex justify-center items-start pt-2 select-none transition-all ${controllerPressed.up ? 'bg-white/40 border-white/50 scale-95' : 'active:bg-white/30'}`}
            >
              ▲
            </button>
            <button 
              onMouseDown={() => handleButtonPress('down', true)}
              onMouseUp={() => handleButtonPress('down', false)}
              onTouchStart={() => handleButtonPress('down', true)}
              onTouchEnd={() => handleButtonPress('down', false)}
              className={`absolute bottom-0 left-1/2 -translate-x-1/2 w-14 h-14 bg-white/10 rounded-b-2xl border border-white/20 backdrop-blur-sm flex justify-center items-end pb-2 select-none transition-all ${controllerPressed.down ? 'bg-white/40 border-white/50 scale-95' : 'active:bg-white/30'}`}
            >
              ▼
            </button>
            <button 
              onMouseDown={() => handleButtonPress('left', true)}
              onMouseUp={() => handleButtonPress('left', false)}
              onTouchStart={() => handleButtonPress('left', true)}
              onTouchEnd={() => handleButtonPress('left', false)}
              className={`absolute left-0 top-1/2 -translate-y-1/2 w-14 h-14 bg-white/10 rounded-l-2xl border border-white/20 backdrop-blur-sm flex justify-start items-center pl-2 select-none transition-all ${controllerPressed.left ? 'bg-white/40 border-white/50 scale-95' : 'active:bg-white/30'}`}
            >
              ◀
            </button>
            <button 
              onMouseDown={() => handleButtonPress('right', true)}
              onMouseUp={() => handleButtonPress('right', false)}
              onTouchStart={() => handleButtonPress('right', true)}
              onTouchEnd={() => handleButtonPress('right', false)}
              className={`absolute right-0 top-1/2 -translate-y-1/2 w-14 h-14 bg-white/10 rounded-r-2xl border border-white/20 backdrop-blur-sm flex justify-end items-center pr-2 select-none transition-all ${controllerPressed.right ? 'bg-white/40 border-white/50 scale-95' : 'active:bg-white/30'}`}
            >
              ▶
            </button>
            <div className="absolute inset-0 m-auto w-14 h-14 bg-white/5 border border-white/10 backdrop-blur-sm rounded-full pointer-events-none"></div>
          </div>
          {/* L1 / L2 */}
          <div className="absolute -top-16 left-2 w-20 h-10 bg-white/10 rounded-xl border border-white/20 active:bg-white/40 backdrop-blur-sm flex items-center justify-center text-white/80 text-sm font-bold tracking-wider select-none shadow-lg">L1</div>
        </div>

        {/* Virtual Gamepad - Right (Action Buttons) */}
        <div className="absolute bottom-12 right-8 w-44 h-44 z-40 opacity-40 hover:opacity-80 transition-opacity">
          <div className="relative w-full h-full mx-auto">
            {/* Action buttons with custom active visual states */}
            <button 
              onMouseDown={() => handleButtonPress('triangle', true)}
              onMouseUp={() => handleButtonPress('triangle', false)}
              onTouchStart={() => handleButtonPress('triangle', true)}
              onTouchEnd={() => handleButtonPress('triangle', false)}
              className={`absolute top-0 left-1/2 -translate-x-1/2 w-14 h-14 bg-white/10 rounded-full border border-white/20 backdrop-blur-sm flex items-center justify-center text-white/90 font-bold text-xl select-none transition-all ${controllerPressed.triangle ? 'bg-green-500/30 border-green-400 scale-95' : 'active:bg-white/30'}`}
            >
              △
            </button>
            <button 
              onMouseDown={() => handleButtonPress('cross', true)}
              onMouseUp={() => handleButtonPress('cross', false)}
              onTouchStart={() => handleButtonPress('cross', true)}
              onTouchEnd={() => handleButtonPress('cross', false)}
              className={`absolute bottom-0 left-1/2 -translate-x-1/2 w-14 h-14 bg-white/10 rounded-full border border-white/20 backdrop-blur-sm flex items-center justify-center text-white/90 font-bold text-xl select-none transition-all ${controllerPressed.cross ? 'bg-blue-500/30 border-blue-400 scale-95 animate-ping-once' : 'active:bg-white/30'}`}
            >
              ✕
            </button>
            <button 
              onMouseDown={() => handleButtonPress('square', true)}
              onMouseUp={() => handleButtonPress('square', false)}
              onTouchStart={() => handleButtonPress('square', true)}
              onTouchEnd={() => handleButtonPress('square', false)}
              className={`absolute left-0 top-1/2 -translate-y-1/2 w-14 h-14 bg-white/10 rounded-full border border-white/20 backdrop-blur-sm flex items-center justify-center text-white/90 font-bold text-xl select-none transition-all ${controllerPressed.square ? 'bg-pink-500/30 border-pink-400 scale-95' : 'active:bg-white/30'}`}
            >
              □
            </button>
            <button 
              onMouseDown={() => handleButtonPress('circle', true)}
              onMouseUp={() => handleButtonPress('circle', false)}
              onTouchStart={() => handleButtonPress('circle', true)}
              onTouchEnd={() => handleButtonPress('circle', false)}
              className={`absolute right-0 top-1/2 -translate-y-1/2 w-14 h-14 bg-white/10 rounded-full border border-white/20 backdrop-blur-sm flex items-center justify-center text-white/90 font-bold text-xl select-none transition-all ${controllerPressed.circle ? 'bg-red-500/30 border-red-400 scale-95' : 'active:bg-white/30'}`}
            >
              ○
            </button>
          </div>
          {/* R1 / R2 */}
          <div className="absolute -top-16 right-2 w-20 h-10 bg-white/10 rounded-xl border border-white/20 active:bg-white/40 backdrop-blur-sm flex items-center justify-center text-white/80 text-sm font-bold tracking-wider select-none shadow-lg">R1</div>
        </div>

        {/* Start / Select Buttons */}
        <div className="absolute bottom-6 left-1/2 -translate-x-1/2 flex space-x-12 z-40 opacity-40 hover:opacity-80 transition-opacity">
          <div className="w-16 h-6 bg-white/10 rounded-full border border-white/20 active:bg-white/40 backdrop-blur-sm flex items-center justify-center text-[10px] text-white/90 tracking-widest shadow-md select-none">SELECT</div>
          <div className="w-16 h-6 bg-white/10 rounded-full border border-white/20 active:bg-white/40 backdrop-blur-sm flex items-center justify-center text-[10px] text-white/90 tracking-widest shadow-md select-none">START</div>
        </div>
      </div>
    );
  }

  // --- ANDROID MENU UI (MATERIAL DESIGN 3) ---
  return (
    <div className="h-screen w-full bg-[#141218] text-[#E6E0E9] font-sans flex flex-col sm:max-w-[420px] sm:mx-auto sm:border-x sm:border-[#38353A] shadow-2xl relative overflow-hidden selection:bg-[#D0BCFF]/30">
      
      {/* Toast Alert pop-up */}
      {toastMessage && (
        <div className="absolute top-16 left-1/2 -translate-x-1/2 bg-[#211F26] border border-[#49454F] text-xs font-medium text-[#E8DEF8] px-4 py-2 rounded-full shadow-lg z-50 flex items-center space-x-2 animate-bounce">
          <Info size={14} className="text-[#D0BCFF]" />
          <span>{toastMessage}</span>
        </div>
      )}

      {/* Material 3 Top Navigation Bar */}
      <div className="bg-[#141218] px-4 py-3 flex items-center justify-between z-10 sticky top-0 border-b border-[#211F26]">
        <div className="flex items-center space-x-3">
          <button className="text-[#CAC4D0] hover:bg-[#49454F]/40 active:bg-[#49454F]/60 p-2 rounded-full -ml-2 transition-colors">
            <Menu size={24} />
          </button>
          <span className="text-xl font-normal tracking-wide text-[#E6E0E9]">AetherSX2</span>
        </div>
        
        {/* Dynamic theme color switcher */}
        <div className="flex items-center space-x-1">
          <div className="flex space-x-1 bg-[#1d1b20] p-1 rounded-full border border-[#49454F]/40 mr-1">
            {['purple', 'emerald', 'amber', 'blue'].map((color) => (
              <button 
                key={color}
                onClick={() => setThemeColor(color)}
                className={`w-4 h-4 rounded-full border border-black/30 transition-all ${
                  color === 'purple' ? 'bg-[#D0BCFF]' :
                  color === 'emerald' ? 'bg-[#34D399]' :
                  color === 'amber' ? 'bg-[#FBBF24]' : 'bg-[#60A5FA]'
                } ${themeColor === color ? 'ring-2 ring-white scale-125' : 'opacity-60'}`}
              />
            ))}
          </div>

          <button 
            onClick={() => setViewMode(viewMode === 'grid' ? 'list' : 'grid')}
            className="text-[#CAC4D0] hover:bg-[#49454F]/40 active:bg-[#49454F]/60 p-2 rounded-full transition-colors"
          >
            {viewMode === 'grid' ? <List size={22} /> : <Grid size={22} />}
          </button>
        </div>
      </div>

      {/* Main Content Area */}
      <div className="flex-1 overflow-y-auto pb-24 custom-scrollbar bg-[#141218]">
        
        {/* Search Bar & Region Filtering (Only on Games library Tab) */}
        {activeTab === 'games' && (
          <div className="px-4 pt-3 pb-2 space-y-3">
            {/* Search Input */}
            <div className="relative">
              <Search className="absolute left-3.5 top-1/2 -translate-y-1/2 text-[#CAC4D0]" size={18} />
              <input 
                type="text"
                placeholder="Search ISO games..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="w-full bg-[#211F26] border border-[#49454F]/30 rounded-2xl py-2.5 pl-11 pr-4 text-sm text-[#E6E0E9] placeholder-[#938F99] focus:outline-none focus:border-[#D0BCFF] focus:ring-1 focus:ring-[#D0BCFF] transition-all"
              />
            </div>

            {/* Region Filters */}
            <div className="flex space-x-2 overflow-x-auto pb-1 scrollbar-none select-none">
              {['All', 'NTSC-U', 'PAL', 'NTSC-J'].map(region => (
                <button
                  key={region}
                  onClick={() => setRegionFilter(region)}
                  className={`text-xs px-3.5 py-1.5 rounded-full font-medium border transition-all shrink-0 ${
                    regionFilter === region 
                      ? `${activeTheme.bgPrimary} ${activeTheme.onPrimary === '#381E72' ? 'text-black' : 'text-white'} border-transparent shadow` 
                      : 'bg-[#211F26] text-[#CAC4D0] border-[#49454F]/40 hover:bg-[#36343b]'
                  }`}
                >
                  {region === 'All' ? 'All Region' : region}
                </button>
              ))}
            </div>
          </div>
        )}

        {/* GAMES TAB VIEW */}
        {activeTab === 'games' && (
          <>
            {filteredGames.length === 0 ? (
              <div className="flex flex-col items-center justify-center p-12 text-center text-[#938F99]">
                <Gamepad2 size={48} className="mb-2 opacity-40 text-[#CAC4D0]" />
                <p className="text-sm">No games matched your query.</p>
              </div>
            ) : (
              <div className={`p-4 pt-2 ${viewMode === 'grid' ? 'grid grid-cols-2 gap-4' : 'flex flex-col space-y-3'}`}>
                {filteredGames.map(game => (
                  <div 
                    key={game.id}
                    onClick={() => startGame(game)}
                    className={`
                      bg-[#1D1B20] rounded-2xl overflow-hidden active:scale-[0.98] transition-transform cursor-pointer shadow-sm relative group border border-transparent hover:border-[#49454F]/50
                      ${viewMode === 'list' ? 'flex items-center p-3' : 'flex flex-col'}
                    `}
                  >
                    {/* Game Cover (Gradient Placeholder) */}
                    <div className={`
                      bg-gradient-to-br ${game.color} flex items-center justify-center relative
                      ${viewMode === 'list' ? 'w-16 h-20 rounded-xl shrink-0' : 'w-full aspect-[3/4]'}
                    `}>
                      <Gamepad2 size={viewMode === 'list' ? 24 : 48} className="text-white/30" />
                      <div className="absolute inset-0 bg-black/5 hover:bg-black/0 transition-colors"></div>
                    </div>

                    {/* Game Info */}
                    <div className={`${viewMode === 'list' ? 'ml-4 flex-1' : 'p-3'} flex flex-col justify-between`}>
                      <div>
                        <h3 className={`font-medium text-[#E6E0E9] ${viewMode === 'grid' && 'line-clamp-2 text-sm h-10 mb-1 leading-snug'}`}>
                          {game.title}
                        </h3>
                        <div className="flex items-center text-[10px] text-[#CAC4D0] font-sans mt-1">
                          <span className="bg-[#332D41] text-[#E8DEF8] px-1.5 py-0.5 rounded mr-2">{game.region}</span>
                          <span>{game.size}</span>
                        </div>
                      </div>
                    </div>

                    {/* Favorite toggle star */}
                    <button 
                      onClick={(e) => toggleFavorite(game.id, e)}
                      className="absolute top-2 right-2 p-1.5 rounded-full bg-black/40 hover:bg-black/60 text-yellow-400 active:scale-90 transition-all z-10"
                    >
                      <span className="text-xs">{game.favorite ? '★' : '☆'}</span>
                    </button>
                  </div>
                ))}
              </div>
            )}
          </>
        )}

        {/* INTERACTIVE SAVE STATES TAB VIEW */}
        {activeTab === 'states' && (
          <div className="p-4 space-y-4">
            <div className="flex justify-between items-center mb-2">
              <span className="text-xs font-semibold uppercase tracking-wider text-[#D0BCFF]">Stored Save States</span>
              <span className="text-xs text-[#938F99] font-mono">{saveStates.length} Slots Busy</span>
            </div>

            {saveStates.length === 0 ? (
              <div className="text-center p-12 bg-[#1d1b20] rounded-3xl border border-[#49454F]/20">
                <Save size={36} className="mx-auto mb-2 text-[#938F99]" />
                <p className="text-sm text-[#CAC4D0]">No active save states found.</p>
                <p className="text-xs text-[#938F99] mt-1">States can be instantly created during live emulations.</p>
              </div>
            ) : (
              <div className="grid grid-cols-1 gap-3">
                {saveStates.map(state => (
                  <div key={state.id} className="bg-[#1D1B20] border border-[#49454F]/30 rounded-2xl overflow-hidden flex shadow-md p-3 relative hover:border-[#D0BCFF]/40 transition-colors">
                    {/* Simulated Capture Slot */}
                    <div className={`w-24 h-16 bg-gradient-to-br ${state.screenshotColor} rounded-xl flex items-center justify-center shrink-0`}>
                      <Save size={24} className="text-white/40" />
                    </div>
                    
                    {/* Info */}
                    <div className="ml-3 flex-1 flex flex-col justify-between">
                      <div>
                        <h4 className="text-xs font-bold text-[#E6E0E9] line-clamp-1">{state.gameTitle}</h4>
                        <p className="text-[11px] text-[#D0BCFF] font-medium mt-0.5">{state.progress}</p>
                      </div>
                      <span className="text-[10px] text-[#938F99] font-mono">{state.timestamp}</span>
                    </div>

                    {/* Delete button */}
                    <button 
                      onClick={(e) => deleteSaveState(state.id, e)}
                      className="absolute bottom-3 right-3 text-[#f43f5e] hover:bg-[#f43f5e]/10 p-2 rounded-full transition-colors"
                      title="Delete State"
                    >
                      <Trash2 size={16} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* SETTINGS TAB VIEW WITH INTEGRATED BIOS CONFIG */}
        {activeTab === 'settings' && (
          <div className="p-2 space-y-4">
            
            {/* Dynamic Custom Theme selector in setting */}
            <div className="px-4 py-3 bg-[#1d1b20] mx-2 rounded-2xl border border-[#49454F]/20">
              <div className="text-sm font-medium text-[#D0BCFF] mb-2 flex items-center">
                <Sliders size={16} className="mr-2" />
                <span>Material You Accent Color</span>
              </div>
              <p className="text-xs text-[#938F99] mb-3">Choose a custom color style for the whole emulator mockup:</p>
              <div className="grid grid-cols-4 gap-2">
                {Object.keys(themes).map(c => (
                  <button 
                    key={c}
                    onClick={() => { setThemeColor(c); triggerToast(`Accent style: ${c.toUpperCase()}`); }}
                    className={`py-1.5 px-2 rounded-xl text-xs font-medium border text-center capitalize transition-all ${
                      themeColor === c ? 'bg-[#332D41] text-[#E8DEF8] border-[#D0BCFF]' : 'bg-[#211F26] text-[#CAC4D0] border-transparent'
                    }`}
                  >
                    {c}
                  </button>
                ))}
              </div>
            </div>

            {/* Bios Configuration Panel */}
            <div className="px-4 pt-1">
              <div className="text-sm font-medium text-[#D0BCFF] mb-2 flex items-center">
                <Cpu size={16} className="mr-2" />
                <span>PS2 System BIOS Files</span>
              </div>
              <p className="text-xs text-[#938F99] mb-3">Please select a valid console BIOS binary:</p>
              
              <div className="space-y-2">
                {biosList.map(bios => (
                  <div 
                    key={bios.id} 
                    onClick={() => selectBios(bios.id)}
                    className={`p-3 bg-[#1D1B20] border rounded-2xl cursor-pointer flex items-center justify-between transition-all ${
                      bios.selected ? `border-[1.5px] ${activeTheme.borderPrimary}` : 'border-[#49454F]/30'
                    }`}
                  >
                    <div className="flex items-center space-x-3">
                      <Folder size={18} className={bios.selected ? activeTheme.textPrimary : 'text-slate-400'} />
                      <div>
                        <div className="text-xs font-semibold text-white">{bios.name}</div>
                        <div className="text-[10px] text-[#938F99] font-mono mt-0.5">{bios.status} • USA Regions</div>
                      </div>
                    </div>
                    {bios.selected && <Check size={16} className={activeTheme.textPrimary} />}
                  </div>
                ))}
              </div>
            </div>

            {/* General Emulator Configuration rows */}
            <div className="px-2">
              <div className="px-4 py-3 text-sm font-medium text-[#D0BCFF]">App Configuration</div>
              <SettingsRow icon={MonitorSmartphone} title="General Engine Settings" subtitle="Boot type, logs, frame limiting" />
              <SettingsRow icon={Cpu} title="EE Core / Affinity" subtitle="Multi-Threaded VU1 recompiler, fast-boot" />
              <SettingsRow icon={Gamepad2} title="Virtual Touch Mapping" subtitle="Configure sizes, locations, and vibrator triggers" />
              <SettingsRow icon={Volume2} title="Audio Decoders" subtitle="Async Mix engines, latency parameters" />
              <SettingsRow icon={Save} title="Memory Cards" subtitle="Format standard .ps2 storage cells" />
            </div>
          </div>
        )}
      </div>

      {/* Material 3 Floating Action Button (FAB) -> Brings up new ISO ROM importing tool */}
      {activeTab === 'games' && (
        <button 
          onClick={() => setShowImportModal(true)}
          className={`absolute bottom-24 right-4 w-14 h-14 ${activeTheme.bgPrimary} ${activeTheme.onPrimary === '#381E72' ? 'text-black' : 'text-white'} hover:brightness-110 active:scale-95 rounded-[1rem] shadow-xl flex items-center justify-center transition-all z-20`}
        >
          <FolderPlus size={26} strokeWidth={2.5} />
        </button>
      )}

      {/* NEW GAME IMPORT MODAL */}
      {showImportModal && (
        <div className="absolute inset-0 bg-black/70 flex items-end justify-center z-50 p-4 animate-fade-in">
          <div className="bg-[#1c1b22] border border-[#38353A] w-full max-w-sm rounded-t-[2rem] p-6 text-white pb-8">
            <div className="flex justify-between items-center mb-4">
              <h3 className="font-bold text-base text-[#D0BCFF]">Add Custom ISO Game</h3>
              <button onClick={() => setShowImportModal(false)} className="text-[#CAC4D0] hover:text-white">✕</button>
            </div>
            
            <form onSubmit={handleImportGame} className="space-y-4">
              <div>
                <label className="text-xs text-[#938F99] block mb-1">Game Title:</label>
                <input 
                  type="text" 
                  value={newGameTitle}
                  onChange={(e) => setNewGameTitle(e.target.value)}
                  placeholder="e.g. Persona 4"
                  className="w-full bg-[#211F26] border border-[#49454F]/50 rounded-xl py-2 px-3 text-sm focus:outline-none focus:border-[#D0BCFF]"
                  required
                />
              </div>

              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="text-xs text-[#938F99] block mb-1">Region:</label>
                  <select 
                    value={newGameRegion}
                    onChange={(e) => setNewGameRegion(e.target.value)}
                    className="w-full bg-[#211F26] border border-[#49454F]/50 rounded-xl py-2 px-3 text-sm focus:outline-none"
                  >
                    <option value="NTSC-U">NTSC-U</option>
                    <option value="PAL">PAL</option>
                    <option value="NTSC-J">NTSC-J</option>
                  </select>
                </div>
                <div>
                  <label className="text-xs text-[#938F99] block mb-1">Size:</label>
                  <input 
                    type="text" 
                    value={newGameSize}
                    onChange={(e) => setNewGameSize(e.target.value)}
                    placeholder="3.2 GB"
                    className="w-full bg-[#211F26] border border-[#49454F]/50 rounded-xl py-2 px-3 text-sm focus:outline-none"
                  />
                </div>
              </div>

              <div>
                <label className="text-xs text-[#938F99] block mb-1">Card Gradient Cover Style:</label>
                <div className="grid grid-cols-3 gap-2">
                  {[
                    { val: "from-indigo-600 to-purple-600", label: "Indigo" },
                    { val: "from-teal-600 to-emerald-600", label: "Emerald" },
                    { val: "from-rose-600 to-amber-600", label: "Sunset" },
                  ].map(option => (
                    <button
                      key={option.val}
                      type="button"
                      onClick={() => setNewGameColor(option.val)}
                      className={`py-1 rounded-lg text-[10px] font-bold border ${newGameColor === option.val ? 'border-white' : 'border-transparent'} bg-gradient-to-r ${option.val}`}
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
              </div>

              <button 
                type="submit"
                className="w-full bg-[#D0BCFF] text-[#381E72] hover:bg-[#E8DEF8] py-2.5 rounded-xl text-sm font-semibold tracking-wide transition-all mt-2"
              >
                Register game to ROM Library
              </button>
            </form>
          </div>
        </div>
      )}

      {/* Material 3 Bottom Navigation Bar */}
      <div className="bg-[#1D1B20] text-[#CAC4D0] pt-2 pb-3 px-2 flex justify-around items-center absolute bottom-0 w-full border-t border-[#38353A]/50 z-10">
        <NavButton icon={Gamepad2} label="Library" id="games" activeTab={activeTab} setActiveTab={setActiveTab} activeTheme={activeTheme} />
        <NavButton icon={Save} label="States" id="states" activeTab={activeTab} setActiveTab={setActiveTab} activeTheme={activeTheme} />
        <NavButton icon={Settings} label="Settings" id="settings" activeTab={activeTab} setActiveTab={setActiveTab} activeTheme={activeTheme} />
      </div>

      {}
      {/* Hide scrollbars for native layout styling */}
      <style dangerouslySetInnerHTML={{__html: `
        .custom-scrollbar::-webkit-scrollbar {
          display: none;
        }
        .custom-scrollbar {
          -ms-overflow-style: none;
          scrollbar-width: none;
        }
        .scrollbar-none::-webkit-scrollbar {
          display: none;
        }
        .scrollbar-none {
          -ms-overflow-style: none;
          scrollbar-width: none;
        }
      `}} />
    </div>
  );
}

// Helper for Bottom Nav Button (Material 3 Pill Style)
function NavButton({ icon: Icon, label, id, activeTab, setActiveTab, activeTheme }) {
  const isActive = activeTab === id;
  return (
    <button 
      onClick={() => setActiveTab(id)}
      className="flex flex-col items-center min-w-[64px] transition-all relative"
    >
      <div className={`px-5 py-1 rounded-full mb-1 transition-all duration-300 ${
        isActive ? `${activeTheme.primaryContainer} ${activeTheme.textPrimary}` : 'text-[#CAC4D0] hover:bg-[#49454F]/40'
      }`}>
        <Icon size={24} strokeWidth={isActive ? 2.5 : 2} />
      </div>
      <span className={`text-[12px] font-medium transition-colors ${isActive ? 'text-[#E6E0E9]' : 'text-[#CAC4D0]'}`}>
        {label}
      </span>
    </button>
  );
}

// Helper for Settings Row
function SettingsRow({ icon: Icon, title, subtitle }) {
  return (
    <div className="flex items-center px-4 py-4 hover:bg-[#49454F]/20 active:bg-[#49454F]/40 cursor-pointer transition-colors border-b border-[#211F26] last:border-0">
      <Icon size={24} className="text-[#CAC4D0] mr-4" />
      <div className="flex-1">
        <div className="text-[#E6E0E9] font-medium text-base">{title}</div>
        <div className="text-[#938F99] text-xs mt-0.5">{subtitle}</div>
      </div>
      <ChevronRight size={16} className="text-[#938F99] ml-2" />
    </div>
  );
}