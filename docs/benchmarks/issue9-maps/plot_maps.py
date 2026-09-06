"""Plot measured routes over ways extracted from the local OSM PBF."""
import json,math
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection,PolyCollection
from matplotlib.lines import Line2D
import matplotlib.patheffects as pe

ROOT=Path(__file__).resolve().parent
WAYS=json.loads((ROOT/'osm-ways.json').read_text())
BEFORE={r['id']:r for r in json.loads((ROOT/'routes-before-osm.json').read_text())}
AFTER={r['id']:r for r in json.loads((ROOT/'routes-after.json').read_text())}
COLORS={'before':'#d65a16','after':'#087d87','start':'#1764c0','end':'#8439a0'}
CONFIG={
58:('Kungstorp','The false zero disappears','Both points were assigned to the same junction.','Two projections on Kämpingevägen, with endpoint access included.', '01-kungstorp-false-zero'),
344:('Vallsta by','A long junction detour becomes shorter','The old lookup counts five street edges around the block.','The projected route uses a short section of the street network.', '02-vallsta-shorter-route'),
186:('Årstaberg','A much longer result that needs review','Both points were assigned to the same junction: no walk counted.','A snaps to an informal dirt path; B snaps to a platform area.', '03-arstaberg-longer-route')}
plt.rcParams.update({'font.family':'DejaVu Sans','font.size':11,'axes.titleweight':'bold','svg.fonttype':'none'})

for ident,(place,headline,oldnote,newnote,stem) in CONFIG.items():
    before,after=BEFORE[ident],AFTER[ident]
    c=before['coordinates'];lon0=(c[0]+c[2])/2;lat0=(c[1]+c[3])/2
    sx=111320*math.cos(math.radians(lat0));sy=111320
    def xy(p):return ((p[0]-lon0)*sx,(p[1]-lat0)*sy)
    start,end=xy(c[:2]),xy(c[2:])
    allcoords=[c[:2],c[2:],before['from_snap'],before['to_snap'],after['from_snap'],after['to_snap']]
    for record in [before,after]:
        for path in record['paths']:allcoords.extend(path)
    cloud=[xy(p) for p in allcoords];xs,ys=zip(*cloud)
    side=max(max(xs)-min(xs),max(ys)-min(ys),140)*1.35
    cx=(min(xs)+max(xs))/2;cy=(min(ys)+max(ys))/2
    bounds=(cx-side/2,cx+side/2,cy-side/2,cy+side/2)
    context=[]
    for w in WAYS:
        chunks=[];chunk=[]
        for n in w['nodes']:
            if n['xy'] is None:
                if len(chunk)>1:chunks.append(chunk)
                chunk=[]
            else:chunk.append(xy(n['xy']))
        if len(chunk)>1:chunks.append(chunk)
        chunks=[p for p in chunks if any(bounds[0]-100<x<bounds[1]+100 and bounds[2]-100<y<bounds[3]+100 for x,y in p)]
        if chunks:context.append((w,chunks))
    def basemap(ax,bounds,inset=False):
        ax.set_facecolor('#f7f8f5')
        roads=[];paths=[];rails=[];buildings=[];greens=[];water=[];platforms=[];labels=[]
        for w,chunks in context:
            t=w['tags']
            if 'building' in t:buildings.extend(p for p in chunks if len(p)>3 and p[0]==p[-1])
            elif t.get('natural')=='water' or t.get('water') or t.get('landuse')=='reservoir':water.extend(p for p in chunks if len(p)>3 and p[0]==p[-1])
            elif t.get('landuse') in ['grass','forest','meadow','recreation_ground'] or t.get('leisure') in ['park','garden']:greens.extend(p for p in chunks if len(p)>3 and p[0]==p[-1])
            if t.get('railway')=='platform' or t.get('public_transport')=='platform':platforms.extend(chunks)
            elif t.get('railway') in ['rail','tram','subway','light_rail']:rails.extend(chunks)
            if t.get('highway'):
                if t['highway'] in ['path','footway','steps','cycleway','pedestrian']:paths.extend(chunks)
                else:roads.extend(chunks)
                if t.get('name'):
                    for p in chunks:
                        inside=[v for v in p if bounds[0]<v[0]<bounds[1] and bounds[2]<v[1]<bounds[3]]
                        if inside:labels.append((t['name'],inside[len(inside)//2]))
        for polygons,color in [(greens,'#e4eddc'),(water,'#dcebf1'),(buildings,'#e1ded6')]:
            if polygons:ax.add_collection(PolyCollection(polygons,facecolors=color,edgecolors='#d2d4cc',linewidths=.4,zorder=0))
        ax.add_collection(LineCollection(roads,colors='#c1c6c6',linewidths=3.6 if not inset else 2.8,zorder=1))
        ax.add_collection(LineCollection(roads,colors='white',linewidths=2.2 if not inset else 1.6,zorder=2))
        ax.add_collection(LineCollection(paths,colors='#9aa99b',linewidths=1.3,linestyles='dashed',zorder=2))
        ax.add_collection(LineCollection(rails,colors='#8b929a',linewidths=1,linestyles='dashdot',zorder=2))
        ax.add_collection(LineCollection(platforms,colors='#b7a5bd',linewidths=1.8,zorder=2))
        used=set();positions=[]
        if not inset:
            for label,pos in labels:
                if label in used or any(math.dist(pos,p)<side*.14 for p in positions):continue
                if len(used)>=7:break
                ax.text(*pos,label,fontsize=8.5,color='#656c73',ha='center',zorder=3,path_effects=[pe.withStroke(linewidth=3,foreground='#f7f8f5')])
                used.add(label);positions.append(pos)
        ax.set_xlim(bounds[:2]);ax.set_ylim(bounds[2:]);ax.set_aspect('equal')
        ax.set_xticks([]);ax.set_yticks([])
        for spine in ax.spines.values():spine.set_color('#cbd3d8')
    def overlay(ax,record,mode,inset=False):
        col=COLORS[mode]
        for path in record['paths']:
            if len(path)<2:continue
            pts=[xy(p) for p in path];x,y=zip(*pts)
            ax.plot(x,y,color='white',lw=6 if not inset else 4.8,zorder=5,solid_capstyle='round')
            ax.plot(x,y,color=col,lw=3.3 if not inset else 2.7,zorder=6,solid_capstyle='round')
        for p,key in [(start,'from_snap'),(end,'to_snap')]:
            snap=xy(record[key]);ax.plot([p[0],snap[0]],[p[1],snap[1]],color=col,lw=1.8,ls=(0,(3,2)),zorder=7)
            ax.scatter(*snap,s=85 if not inset else 55,marker='D',facecolor='white',edgecolor=col,lw=1.8,zorder=8)
        for p,label,color,offset in [(start,'A',COLORS['start'],(-17,8)),(end,'B',COLORS['end'],(9,-17))]:
            ax.scatter(*p,s=105 if not inset else 70,facecolor=color,edgecolor='white',lw=1.5,zorder=10)
            ax.annotate(label,p,xytext=offset,textcoords='offset points',fontsize=13,fontweight='bold',color=color,zorder=12,path_effects=[pe.withStroke(linewidth=3,foreground='white')])
    fig,axes=plt.subplots(1,2,figsize=(16,9.6))
    fig.subplots_adjust(left=.04,right=.96,top=.83,bottom=.25,wspace=.065)
    fig.patch.set_facecolor('white')
    fig.text(.04,.948,f'{place}  |  {headline}',fontsize=23,fontweight='bold',color='#152936')
    fig.text(.04,.91,f'Actual OSM geometry · benchmark pair #{ident} · A and B are about 50 m apart',fontsize=12,color='#526371')
    for ax,mode,record,note in zip(axes,['before','after'],[before,after],[oldnote,newnote]):
        basemap(ax,bounds);overlay(ax,record,mode)
        ax.set_title(f'{mode.upper()}   {record["seconds"]} seconds',fontsize=19,color=COLORS[mode],pad=15,loc='left')
        ax.text(0,-.055,note,transform=ax.transAxes,fontsize=10.5,color='#263746',va='top',wrap=True)
        scale=next(s for s in [500,200,100,50,20,10] if s<=side*.25)
        x0=bounds[0]+side*(.58 if side>300 else .06);y0=bounds[2]+side*.07
        ax.plot([x0,x0+scale],[y0,y0],color='#24323d',lw=3,zorder=9)
        ax.text(x0+scale/2,y0+side*.02,f'{scale} m',ha='center',fontsize=9,zorder=9,bbox=dict(facecolor='white',edgecolor='none',alpha=.85,pad=1))
        ax.annotate('N',xy=(.955,.92),xytext=(.955,.84),xycoords='axes fraction',ha='center',fontsize=10,arrowprops=dict(arrowstyle='-|>',color='#34414c'),color='#34414c',zorder=20)
        if side>300:
            inset=ax.inset_axes([.03,.04,.36,.36],zorder=30);ib=(-80,80,-80,80)
            basemap(inset,ib,True);overlay(inset,record,mode,True)
            inset.set_title('Endpoint detail · same coordinates',fontsize=7,pad=3)
    handles=[Line2D([0],[0],marker='o',ls='',color=COLORS['start'],label='A: query origin'),Line2D([0],[0],marker='o',ls='',color=COLORS['end'],label='B: query destination'),Line2D([0],[0],marker='D',ls='',mfc='white',mec='#555',label='Assigned node / edge projection'),Line2D([0],[0],color='#555',lw=3,label='Counted network walk'),Line2D([0],[0],color='#555',ls='--',label='Snap connection')]
    fig.legend(handles=handles,loc='lower left',bbox_to_anchor=(.033,.12),ncol=3,frameon=False,fontsize=10)
    fig.text(.04,.095,'Dashed connections are omitted from the old direct-walking cost and included in the new cost.',fontsize=10,color='#526371')
    detail='Årstaberg: A → OSM way 1217029049 (informal path); B → way 243561938 (platform area). Correct attachment is not established.' if ident==186 else ('Vallsta: A → way 707455550 (gravel road); B → way 4495317 (road 83).' if ident==344 else 'Kungstorp: both new projections lie on OSM way 431574592, Kämpingevägen.')
    fig.text(.04,.057,detail,fontsize=9.2,color='#526371')
    fig.text(.04,.025,f'Local Sweden OSM extract · © OpenStreetMap contributors · To: {c[3]:.6f}, {c[2]:.6f} · Traced costs checked against routing output',fontsize=8.5,color='#6a7781')
    fig.savefig(ROOT/(stem+'.png'),dpi=180,facecolor='white')
    fig.savefig(ROOT/(stem+'.svg'),facecolor='white')
    plt.close(fig)
print('Saved three PNG maps and editable SVG versions.')
